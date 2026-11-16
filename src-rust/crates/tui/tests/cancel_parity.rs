//! Classic and Next cancel a running turn the same way (#408).
//!
//! Escape and Ctrl+C ask the interactive loop to cancel the turn through its
//! token, the cancel stays pending until the worker has stopped, and output
//! still queued from the cancelled turn cannot restart it. Each trace is
//! replayed into an App per renderer, drawn with that renderer after every
//! step the way the interactive loop draws, and both must reach the same
//! semantic state after every step:
//! - the durable messages;
//! - the live tool rows by id: name, input, the result preview the row holds,
//!   and status, which tells running, done and error apart;
//! - `is_streaming` and the cancellation phase;
//! - the operations the loop performs next: the presentation effects, the
//!   cancel request it takes, and whether a key took its interrupt path,
//!   which cancels the token as well.
//!
//! Presentation-only, so not compared:
//! - Rows of the calls a cancel interrupts, and anything later shown for
//!   those calls. Classic drops the rows; Next keeps them in its interrupted
//!   list, where what the call reports later still reaches them. They are
//!   compared only as the calls each App has shown a row for, live or not.
//!   Rows the backlog starts afterwards are live rows like any other and are
//!   compared.
//! - Status wording and the spinner verb.

use std::collections::{BTreeMap, BTreeSet};

use claurst_api::streaming::ContentDelta;
use claurst_api::AnthropicStreamEvent;
use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::types::Message;
use claurst_query::QueryEvent;
use claurst_tui::app::{App, ToolStatus, ToolUseBlock};
use claurst_tui::application::{
    PresentationEffect, ScopedQueryEvent, SessionScope, TurnCancellation, UiRenderer,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn delta(text: &str) -> QueryEvent {
    QueryEvent::Stream(AnthropicStreamEvent::ContentBlockDelta {
        index: 0,
        delta: ContentDelta::TextDelta {
            text: text.to_string(),
        },
    })
}

fn tool_start(id: &str) -> QueryEvent {
    QueryEvent::ToolStart {
        tool_name: "Bash".to_string(),
        tool_id: id.to_string(),
        input_json: r#"{"command":"sleep 30"}"#.to_string(),
    }
}

fn tool_end(id: &str) -> QueryEvent {
    QueryEvent::ToolEnd {
        tool_name: "Bash".to_string(),
        tool_id: id.to_string(),
        result: "cancelled".to_string(),
        is_error: true,
    }
}

fn escape() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}

fn ctrl_c() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
}

#[derive(Clone)]
enum Step {
    Event(QueryEvent),
    /// A key, routed the way the interactive loop routes it: the interrupt and
    /// exit keys first, then the App's own handling.
    Key(KeyEvent),
    /// Remote control cancelled; the loop has cancelled the token of the turn
    /// running, if one is.
    RemoteCancel {
        turn_running: bool,
    },
    /// No worker is running any more, so the loop settles a pending cancel.
    Settle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolObservation {
    name: String,
    input: String,
    /// The preview the row holds; the full result is in the durable messages.
    result: Option<String>,
    status: ToolStatus,
}

/// The state both renderers must agree on after a step.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Semantic {
    /// The durable conversation, serialized.
    messages: String,
    /// Live tool rows by id, except those of calls a cancel interrupted.
    tools: BTreeMap<String, ToolObservation>,
    /// Every call the App has shown a row for so far, live, interrupted or
    /// dropped.
    observed: BTreeSet<String>,
    is_streaming: bool,
    cancellation: TurnCancellation,
    /// Operations the interactive loop performs after this step.
    effects: Vec<PresentationEffect>,
    cancel_requested: bool,
    /// The key took the loop's interrupt path, which cancels the turn's token.
    interrupt_key: bool,
}

fn draw(app: &App, renderer: UiRenderer) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("test terminal");
    terminal
        .draw(|frame| match renderer {
            UiRenderer::Classic => claurst_tui::render::render_app(frame, app),
            UiRenderer::Next => claurst_tui::next::render_next(frame, app),
        })
        .expect("draw frame");
    let buffer = terminal.backend().buffer();
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

struct Replay {
    renderer: UiRenderer,
    app: App,
    scope: SessionScope,
    observed: BTreeSet<String>,
    /// Calls whose rows were live when a cancel was asked for.
    interrupted: BTreeSet<String>,
    steps: Vec<Semantic>,
    /// The frame drawn after each step.
    frames: Vec<String>,
}

impl Replay {
    fn new(renderer: UiRenderer) -> Self {
        let config = Config {
            model: Some("claude-baseline-model".to_string()),
            provider: Some("anthropic".to_string()),
            ..Config::default()
        };
        let mut app = App::new(config, CostTracker::new());
        app.has_credentials = true;
        app.select_renderer(renderer);
        // Without this the comparison would hold trivially: two Classic Apps.
        assert_eq!(
            app.renderer, renderer,
            "the replay runs the renderer under test"
        );
        app.push_message(Message::user("Run the slow command.".to_string()));
        Self {
            renderer,
            app,
            scope: SessionScope::default(),
            observed: BTreeSet::new(),
            interrupted: Default::default(),
            steps: Vec::new(),
            frames: Vec::new(),
        }
    }

    /// Apply each step, draw, then hand a cancel request to the loop the way
    /// the interactive loop does every pass.
    fn run(mut self, steps: Vec<Step>) -> Self {
        for step in steps {
            let mut effects = Vec::new();
            let mut interrupt_key = false;
            let was_idle = self.app.cancellation == TurnCancellation::Idle;
            let live_before: Vec<String> = self
                .app
                .tool_use_blocks
                .iter()
                .map(|block| block.id.clone())
                .collect();
            match step {
                Step::Event(event) => {
                    effects = self.scope.apply(
                        &mut self.app,
                        ScopedQueryEvent {
                            generation: self.scope.current(),
                            event,
                        },
                    );
                }
                Step::Key(key) => match self.app.handle_control_key(key) {
                    Some(interrupt) => interrupt_key = interrupt,
                    None => {
                        self.app.handle_key_event(key);
                    }
                },
                Step::RemoteCancel { turn_running } => {
                    self.app.note_cancellation_sent(turn_running)
                }
                Step::Settle => self.app.settle_cancellation(),
            }
            self.frames.push(draw(&self.app, self.renderer));
            if was_idle && self.app.cancellation != TurnCancellation::Idle {
                self.interrupted.extend(live_before);
            }
            let cancel_requested = self.app.take_cancellation_request();
            let observe = |block: &ToolUseBlock| {
                (
                    block.id.clone(),
                    ToolObservation {
                        name: block.name.clone(),
                        input: block.input_json.clone(),
                        result: block.output_preview.clone(),
                        status: block.status.clone(),
                    },
                )
            };
            let live: BTreeMap<_, _> = self.app.tool_use_blocks.iter().map(observe).collect();
            self.observed.extend(
                self.app
                    .tool_use_blocks
                    .iter()
                    .chain(self.app.interrupted_tools.iter())
                    .map(|block| block.id.clone()),
            );
            self.steps.push(Semantic {
                messages: serde_json::to_string(&*self.app.messages).expect("messages serialize"),
                tools: live
                    .into_iter()
                    .filter(|(id, _)| !self.interrupted.contains(id))
                    .collect(),
                observed: self.observed.clone(),
                is_streaming: self.app.is_streaming,
                cancellation: self.app.cancellation,
                effects,
                cancel_requested,
                interrupt_key,
            });
        }
        self
    }
}

/// Replay `steps` for both renderers; the semantic state must match after
/// every step.
fn assert_parity(name: &str, steps: Vec<Step>) -> (Replay, Replay) {
    compare(
        name,
        Replay::new(UiRenderer::Classic).run(steps.clone()),
        Replay::new(UiRenderer::Next).run(steps),
    )
}

/// The semantic state of two finished replays must match after every step.
fn compare(name: &str, classic: Replay, next: Replay) -> (Replay, Replay) {
    assert_eq!(classic.steps.len(), next.steps.len(), "{name}");
    for (index, (classic_step, next_step)) in classic.steps.iter().zip(&next.steps).enumerate() {
        assert_eq!(
            classic_step, next_step,
            "{name}: renderers differ after step {index}"
        );
    }
    (classic, next)
}

/// Escape mid-answer, then the cancelled turn's backlog: more text, a tool
/// start and its result, and the partial answer as a durable message.
fn escape_then_backlog() -> Vec<Step> {
    let mut partial = Message::assistant("partial answer ");
    partial.uuid = Some("msg-cancel-partial".to_string());
    vec![
        Step::Event(delta("partial answer ")),
        Step::Key(escape()),
        Step::Event(delta("more from the backlog")),
        Step::Event(tool_start("toolu-cancel-backlog")),
        Step::Event(tool_end("toolu-cancel-backlog")),
        Step::Event(QueryEvent::DurableMessage { message: partial }),
        Step::Settle,
        Step::Key(ctrl_c()),
    ]
}

/// Ctrl+C while a tool runs, then the backlog: text and the tool's result.
fn ctrl_c_during_a_tool() -> Vec<Step> {
    vec![
        Step::Event(delta("running it ")),
        Step::Event(tool_start("toolu-cancel-running")),
        Step::Key(ctrl_c()),
        Step::Event(delta("late text")),
        Step::Event(tool_end("toolu-cancel-running")),
        Step::Settle,
    ]
}

/// A remote-control cancel while a tool runs, then the backlog: text, a tool
/// that never ends, one that does, and one that reports finishing before its
/// result arrives.
fn remote_cancel_during_a_tool() -> Vec<Step> {
    vec![
        Step::Event(delta("running it ")),
        Step::Event(tool_start("toolu-cancel-remote")),
        Step::RemoteCancel { turn_running: true },
        Step::Event(delta("late text")),
        Step::Event(tool_start("toolu-cancel-remote-backlog")),
        Step::Event(tool_start("toolu-cancel-remote-done")),
        Step::Event(tool_end("toolu-cancel-remote-done")),
        Step::Event(tool_start("toolu-cancel-remote-reported")),
        Step::Event(QueryEvent::ToolFinished {
            tool_id: "toolu-cancel-remote-reported".to_string(),
            is_error: false,
        }),
        Step::Settle,
    ]
}

/// Ctrl+C while a tool runs, then the backlog repeats that call's start and
/// reports how it ended.
fn ctrl_c_then_a_repeated_start() -> Vec<Step> {
    vec![
        Step::Event(tool_start("toolu-cancel-repeated")),
        Step::Key(ctrl_c()),
        Step::Event(tool_start("toolu-cancel-repeated")),
        Step::Event(QueryEvent::ToolFinished {
            tool_id: "toolu-cancel-repeated".to_string(),
            is_error: true,
        }),
        Step::Event(tool_end("toolu-cancel-repeated")),
        Step::Settle,
    ]
}

/// The turn completes in the backlog: the runtime had finished before the
/// cancel reached it.
fn escape_then_a_completed_turn() -> Vec<Step> {
    vec![
        Step::Event(delta("the whole answer")),
        Step::Key(escape()),
        Step::Event(QueryEvent::TurnComplete {
            turn: 1,
            stop_reason: "end_turn".to_string(),
            usage: None,
        }),
        Step::Settle,
    ]
}

/// A replay whose App rings and toasts on every completed turn, however short.
fn announcing(renderer: UiRenderer) -> Replay {
    let mut replay = Replay::new(renderer);
    replay.app.completion_toast_enabled = true;
    replay.app.completion_toast_min_secs = 0;
    replay.app.bell_on_complete = true;
    replay
}

#[test]
fn escape_cancels_the_turn_and_its_backlog_cannot_restart_it_in_either_renderer() {
    let (classic, next) = assert_parity("escape then backlog", escape_then_backlog());
    let steps = &classic.steps;
    assert!(steps[1].cancel_requested, "Escape asks the loop to cancel");
    for (index, step) in steps.iter().enumerate().take(6).skip(1) {
        assert_eq!(
            step.cancellation,
            TurnCancellation::AwaitingRuntime,
            "pending until the worker stops (step {index})"
        );
        assert!(
            !step.is_streaming,
            "the backlog restarted the turn (step {index})"
        );
    }
    assert!(steps[5].messages.contains("msg-cancel-partial"));
    assert_eq!(
        steps[5].tools["toolu-cancel-backlog"].result.as_deref(),
        Some("cancelled")
    );
    assert_eq!(steps[6].cancellation, TurnCancellation::Idle);
    // Settled, the next Ctrl+C starts the exit sequence instead of cancelling.
    assert!(!steps[7].cancel_requested);
    for replay in [&classic, &next] {
        assert_eq!(
            replay.app.exit_key_sequence_start,
            Some('c'),
            "{:?}",
            replay.renderer
        );
    }
}

#[test]
fn ctrl_c_cancels_a_running_tool_the_same_way_in_either_renderer() {
    let (classic, next) = assert_parity("ctrl+c during a tool", ctrl_c_during_a_tool());
    assert!(
        classic.steps[2].cancel_requested,
        "Ctrl+C asks the loop to cancel"
    );
    assert!(classic.steps[2..5].iter().all(|step| !step.is_streaming));
    assert!(classic.steps[2..5]
        .iter()
        .all(|step| step.cancellation == TurnCancellation::AwaitingRuntime));
    assert_eq!(classic.steps[5].cancellation, TurnCancellation::Idle);
    // Presentation only: Classic clears its tool rows on cancel, as it always
    // has; Next keeps the interrupted call.
    assert!(classic.app.tool_use_blocks.is_empty());
    assert!(classic.app.interrupted_tools.is_empty());
    assert!(next
        .app
        .interrupted_tools
        .iter()
        .any(|block| block.id == "toolu-cancel-running"));
}

#[test]
fn classic_says_cancelling_until_the_worker_stops_then_cancelled() {
    let classic = Replay::new(UiRenderer::Classic).run(vec![
        Step::Event(delta("partial answer ")),
        Step::Key(escape()),
        Step::Event(delta("more from the backlog")),
        Step::Settle,
    ]);
    for (index, frame) in classic.frames[1..3].iter().enumerate() {
        assert!(
            frame.contains("Cancelling…"),
            "step {}:\n{frame}",
            index + 1
        );
        assert!(
            !frame.contains("Cancelled."),
            "step {}:\n{frame}",
            index + 1
        );
    }
    let settled = &classic.frames[3];
    assert!(settled.contains("Cancelled."), "{settled}");
    assert!(!settled.contains("Cancelling"), "{settled}");
}

#[test]
fn a_remote_cancel_follows_the_same_model_in_either_renderer() {
    let (classic, next) = assert_parity("remote cancel", remote_cancel_during_a_tool());
    // The loop already cancelled the token, so nothing is requested.
    assert!(classic.steps.iter().all(|step| !step.cancel_requested));
    assert!(classic.steps[2..7]
        .iter()
        .all(|step| !step.is_streaming && step.cancellation == TurnCancellation::AwaitingRuntime));
    assert!(classic.steps[4]
        .tools
        .contains_key("toolu-cancel-remote-backlog"));
    assert_eq!(classic.steps[9].cancellation, TurnCancellation::Idle);
    // Classic shows the remote cancel as pending until the settle.
    for (index, frame) in classic.frames.iter().enumerate().take(9).skip(2) {
        assert!(frame.contains("Cancelling…"), "step {index}:\n{frame}");
    }
    assert!(
        classic.frames[9].contains("Cancelled."),
        "{}",
        classic.frames[9]
    );
    // Settling interrupts the backlog tool that never ended; the one that
    // ended and the one that reported finishing keep their rows.
    for replay in [&classic, &next] {
        let rows: Vec<_> = replay
            .app
            .tool_use_blocks
            .iter()
            .map(|block| (block.id.as_str(), block.run.finished.is_some()))
            .collect();
        assert_eq!(
            rows,
            [
                ("toolu-cancel-remote-done", true),
                ("toolu-cancel-remote-reported", true)
            ],
            "{:?}",
            replay.renderer
        );
        assert!(replay
            .app
            .interrupted_tools
            .iter()
            .any(|block| block.id == "toolu-cancel-remote-backlog"));
    }
}

#[test]
fn a_remote_cancel_with_no_turn_running_only_reports_it() {
    let (classic, next) = assert_parity(
        "remote cancel while idle",
        vec![Step::RemoteCancel {
            turn_running: false,
        }],
    );
    let step = &classic.steps[0];
    assert_eq!(step.cancellation, TurnCancellation::Idle);
    assert!(!step.is_streaming && !step.cancel_requested);
    for replay in [&classic, &next] {
        assert_eq!(
            replay.app.status_message.as_deref(),
            Some("Cancelled by remote control."),
            "{:?}",
            replay.renderer
        );
    }
    assert!(
        !classic.frames[0].contains("Cancelling"),
        "{}",
        classic.frames[0]
    );
}

#[test]
fn a_turn_completed_in_the_backlog_still_settles_as_cancelled() {
    let (classic, next) = compare(
        "completed in the backlog",
        announcing(UiRenderer::Classic).run(escape_then_a_completed_turn()),
        announcing(UiRenderer::Next).run(escape_then_a_completed_turn()),
    );
    assert!(classic.steps[1].cancel_requested);
    assert!(
        classic.steps[2].effects.is_empty(),
        "a cancelled turn rings no bell"
    );
    for replay in [&classic, &next] {
        assert!(
            replay
                .app
                .notifications
                .notifications
                .iter()
                .all(|notification| !notification.message.starts_with("Turn done")),
            "{:?}: a cancelled turn shows no completion toast",
            replay.renderer
        );
        let settled = &replay.frames[3];
        assert!(
            settled.contains("Cancelled"),
            "{:?}:\n{settled}",
            replay.renderer
        );
    }
    // Pending, Classic says so over the done marker the completion set.
    assert!(
        classic.frames[2].contains("Cancelling…") && !classic.frames[2].contains("· done"),
        "{}",
        classic.frames[2]
    );
    assert!(
        !classic.frames[3].contains("· done"),
        "{}",
        classic.frames[3]
    );
    // Without the cancel, the same completion rings and toasts.
    let completed = announcing(UiRenderer::Classic).run(vec![
        Step::Event(delta("the whole answer")),
        Step::Event(QueryEvent::TurnComplete {
            turn: 1,
            stop_reason: "end_turn".to_string(),
            usage: None,
        }),
    ]);
    assert_eq!(completed.steps[1].effects, [PresentationEffect::RingBell]);
    assert!(
        completed.frames[1].contains("· done"),
        "{}",
        completed.frames[1]
    );
}

#[test]
fn a_repeated_start_of_an_interrupted_call_cannot_restart_the_turn() {
    let (classic, next) = assert_parity("repeated start", ctrl_c_then_a_repeated_start());
    assert!(
        classic.steps[1].interrupt_key,
        "Ctrl+C takes the interrupt path"
    );
    assert!(classic.steps[1].cancel_requested);
    assert!(
        !classic.steps[2].is_streaming,
        "the repeat restarted the turn"
    );
    assert_eq!(classic.steps[5].cancellation, TurnCancellation::Idle);
    for replay in [&classic, &next] {
        assert!(
            replay
                .app
                .tool_use_blocks
                .iter()
                .all(|block| block.status != ToolStatus::Running),
            "{:?}",
            replay.renderer
        );
    }
    // Next keeps the call's one interrupted row, and what the call reports
    // afterwards reaches it.
    let rows: Vec<_> = next
        .app
        .interrupted_tools
        .iter()
        .map(|block| block.id.as_str())
        .collect();
    assert_eq!(rows, ["toolu-cancel-repeated"]);
    let row = &next.app.interrupted_tools[0];
    assert_eq!(row.output_preview.as_deref(), Some("cancelled"));
    assert!(row.run.finished.is_some_and(|(_, is_error)| is_error));
}
