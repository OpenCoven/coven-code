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
//!   list. They are compared only as what each App observed: every row it has
//!   shown, live or not. Rows the backlog starts afterwards are live rows like
//!   any other and are compared.
//! - Status wording and the spinner verb.

use std::collections::BTreeMap;

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
    /// Remote control cancelled the turn; the loop has cancelled its token.
    RemoteCancel,
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
    /// Every row the App has shown so far, live, interrupted or dropped.
    observed: BTreeMap<String, ToolObservation>,
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
    observed: BTreeMap<String, ToolObservation>,
    /// Calls whose rows were live when a cancel was asked for.
    interrupted: std::collections::BTreeSet<String>,
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
            observed: BTreeMap::new(),
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
                Step::RemoteCancel => self.app.note_cancellation_sent(true),
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
            self.observed.extend(live.clone());
            self.observed
                .extend(self.app.interrupted_tools.iter().map(observe));
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
    let classic = Replay::new(UiRenderer::Classic).run(steps.clone());
    let next = Replay::new(UiRenderer::Next).run(steps);
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
/// that never ends and one that does.
fn remote_cancel_during_a_tool() -> Vec<Step> {
    vec![
        Step::Event(delta("running it ")),
        Step::Event(tool_start("toolu-cancel-remote")),
        Step::RemoteCancel,
        Step::Event(delta("late text")),
        Step::Event(tool_start("toolu-cancel-remote-backlog")),
        Step::Event(tool_start("toolu-cancel-remote-done")),
        Step::Event(tool_end("toolu-cancel-remote-done")),
        Step::Settle,
    ]
}

/// Ctrl+C while a tool runs, then the backlog repeats that call's start.
fn ctrl_c_then_a_repeated_start() -> Vec<Step> {
    vec![
        Step::Event(tool_start("toolu-cancel-repeated")),
        Step::Key(ctrl_c()),
        Step::Event(tool_start("toolu-cancel-repeated")),
        Step::Settle,
    ]
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
    assert_eq!(classic.steps[7].cancellation, TurnCancellation::Idle);
    // Settling interrupts the backlog tool that never ended; the one that
    // ended keeps its row.
    for replay in [&classic, &next] {
        let rows: Vec<_> = replay
            .app
            .tool_use_blocks
            .iter()
            .map(|block| (block.id.as_str(), block.status.clone()))
            .collect();
        assert_eq!(
            rows,
            [("toolu-cancel-remote-done", ToolStatus::Error)],
            "{:?}",
            replay.renderer
        );
    }
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
    assert_eq!(classic.steps[3].cancellation, TurnCancellation::Idle);
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
    // Next keeps the call's one interrupted row.
    let rows: Vec<_> = next
        .app
        .interrupted_tools
        .iter()
        .map(|block| block.id.as_str())
        .collect();
    assert_eq!(rows, ["toolu-cancel-repeated"]);
}
