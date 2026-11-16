//! Classic/Next parity over the shared presentation seam.
//!
//! Each synthetic runtime trace and key sequence is replayed into an App per
//! renderer, with the renderer selected the way the interactive loop selects
//! it, so the reducer's renderer-gated branches run for the side under test.
//! Both must reach the same semantic state and send the same operations, and
//! both renderers must show the semantic content of that state.
//!
//! Semantic, compared after every step of a trace:
//! - the durable `App::messages`;
//! - the live tool rows by id: name, input, the result preview the row holds,
//!   and status, which tells running, done and error apart (full results
//!   reach the durable messages);
//! - `is_streaming` and the cancellation phase;
//! - the operations the loop performs next: the `PresentationEffect` list,
//!   the `take_cancellation_request` result, and whether a key took the
//!   loop's interrupt path, which cancels the turn's token.
//!
//! A key sequence is compared once at its end: the `take_input` submission,
//! the permission decisions and the whole projected `TranscriptView`, which on
//! that path includes status and tool rows.
//!
//! Presentation-only, so allowed to differ:
//! - Rows a cancel interrupts. Classic drops them; Next keeps them in its
//!   interrupted list. They are compared only as what each App observed: every
//!   row it has shown, live or not.
//! - Status wording, the spinner verb, and Next's shell and inspector state.
//!
//! Semantic, but different by design (#411): how a permission request opens.
//! Next focuses deny and ignores keys other than Escape until
//! `PERMISSION_ARM_DELAY` of quiet has passed; Classic opens on the first
//! option and takes keys at once. So the same relative keys (Up, Down, Enter)
//! decide differently, and on Next keys pressed as the request opens decide
//! nothing. `permission_requests_open_differently_by_design` pins those
//! outcomes; the outgoing-operations comparison uses keys that choose an
//! option explicitly, pressed once the request is armed.
//!
//! Cancelling is not among the compared traces. On this base Classic's Escape
//! does not ask the loop to cancel the turn, and Classic records no pending
//! cancel for Ctrl+C either, although the loop's interrupt path cancels the
//! token. So a cancel trace differs in `cancel_requested`, the cancellation
//! phase and `is_streaming`. That is #408 decision D1, which moves Classic to
//! Next's cancel model; the Next side is covered by
//! `next_backlog_tool_start_does_not_restart_a_cancelled_turn`.

use std::collections::BTreeMap;

use claurst_api::streaming::ContentDelta;
use claurst_api::AnthropicStreamEvent;
use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::types::{ContentBlock, Message, ToolResultContent};
use claurst_query::QueryEvent;
use claurst_tui::app::{App, ToolStatus, ToolUseBlock};
use claurst_tui::application::view::TranscriptView;
use claurst_tui::application::{
    PresentationEffect, ScopedQueryEvent, SessionGeneration, SessionScope, TurnCancellation,
    UiRenderer,
};
use claurst_tui::dialogs::{handle_permission_key_at, PermissionRequest, PERMISSION_ARM_DELAY};
use claurst_tui::NotificationKind;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn fixture_app(renderer: UiRenderer) -> App {
    let config = Config {
        model: Some("claude-baseline-model".to_string()),
        provider: Some("anthropic".to_string()),
        ..Config::default()
    };
    let mut app = App::new(config, CostTracker::new());
    app.has_credentials = true;
    app.select_renderer(renderer);
    app
}

fn delta(text: &str) -> QueryEvent {
    QueryEvent::Stream(AnthropicStreamEvent::ContentBlockDelta {
        index: 0,
        delta: ContentDelta::TextDelta {
            text: text.to_string(),
        },
    })
}

fn tool_start(id: &str, name: &str, input_json: &str) -> QueryEvent {
    QueryEvent::ToolStart {
        tool_name: name.to_string(),
        tool_id: id.to_string(),
        input_json: input_json.to_string(),
    }
}

fn tool_end(id: &str, name: &str, result: &str, is_error: bool) -> QueryEvent {
    QueryEvent::ToolEnd {
        tool_name: name.to_string(),
        tool_id: id.to_string(),
        result: result.to_string(),
        is_error,
    }
}

fn turn_complete(stop_reason: &str) -> QueryEvent {
    QueryEvent::TurnComplete {
        turn: 1,
        stop_reason: stop_reason.to_string(),
        usage: None,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// One step of what the interactive loop feeds the App.
#[derive(Clone)]
enum Step {
    /// An observation from the current generation's worker.
    Event(QueryEvent),
    /// A new session generation starts, as when the conversation is cleared,
    /// rewound or resumed. Only the generation changes; the App keeps its
    /// state, so this exercises the filtering of the old generation's events.
    Advance,
    /// An observation from a worker of the generation before the current one.
    Stale(QueryEvent),
    /// A key, routed the way the interactive loop routes it: the interrupt and
    /// exit keys first, then the App's own handling.
    Key(KeyEvent),
    /// No worker is running any more, so the loop settles a pending cancel.
    Settle,
}

/// What a renderer was told about one tool call.
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
    /// Live tool rows by id.
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

/// Replays steps into one App the way the interactive loop does: apply the
/// step, draw with the renderer under test, then hand any cancel request on.
struct Replay {
    renderer: UiRenderer,
    app: App,
    scope: SessionScope,
    previous: SessionGeneration,
    observed: BTreeMap<String, ToolObservation>,
    steps: Vec<Semantic>,
}

impl Replay {
    fn new(renderer: UiRenderer) -> Self {
        let mut app = fixture_app(renderer);
        // Without this the comparison would hold trivially: two Classic Apps.
        assert_eq!(
            app.renderer, renderer,
            "the replay runs the renderer under test"
        );
        app.push_message(Message::user("Summarize the engine contract.".to_string()));
        app.turn_start = None;
        let scope = SessionScope::default();
        let previous = scope.current();
        Self {
            renderer,
            app,
            scope,
            previous,
            observed: BTreeMap::new(),
            steps: Vec::new(),
        }
    }

    fn run(mut self, steps: impl IntoIterator<Item = Step>) -> Self {
        for step in steps {
            let mut effects = Vec::new();
            let mut interrupt_key = false;
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
                Step::Advance => {
                    self.previous = self.scope.current();
                    self.scope.advance();
                }
                Step::Stale(event) => {
                    effects = self.scope.apply(
                        &mut self.app,
                        ScopedQueryEvent {
                            generation: self.previous,
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
                Step::Settle => self.app.settle_cancellation(),
            }
            draw(&self.app, self.renderer, 120, 40);
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
            let tools: BTreeMap<_, _> = self.app.tool_use_blocks.iter().map(observe).collect();
            self.observed.extend(tools.clone());
            self.observed
                .extend(self.app.interrupted_tools.iter().map(observe));
            self.steps.push(Semantic {
                messages: serde_json::to_string(&*self.app.messages).expect("messages serialize"),
                tools,
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

    fn last(&self) -> &Semantic {
        self.steps.last().expect("at least one step")
    }
}

fn from_events(events: Vec<QueryEvent>) -> Vec<Step> {
    events.into_iter().map(Step::Event).collect()
}

/// A plain turn: text, one tool call and its result, more text, a status.
fn plain_turn() -> Vec<Step> {
    from_events(vec![
        delta("Reading the contract "),
        tool_start(
            "toolu-parity-1",
            "Read",
            r#"{"file_path":"ENGINE-CONTRACT.md"}"#,
        ),
        tool_end("toolu-parity-1", "Read", "contract text", false),
        delta("now."),
        QueryEvent::Status("Thinking about the reply".to_string()),
    ])
}

/// Output from the worker of a replaced conversation arrives late.
fn stale_generation() -> Vec<Step> {
    vec![
        Step::Event(delta("before the clear ")),
        Step::Event(tool_start(
            "toolu-parity-old",
            "Bash",
            r#"{"command":"cargo test"}"#,
        )),
        Step::Advance,
        Step::Stale(delta("late")),
        Step::Stale(tool_start(
            "toolu-parity-stale",
            "Bash",
            r#"{"command":"rm -rf target"}"#,
        )),
        Step::Stale(tool_end("toolu-parity-old", "Bash", "stale result", true)),
        Step::Stale(QueryEvent::Error("stale failure".to_string())),
        Step::Stale(QueryEvent::DurableMessage {
            message: Message::assistant("stale answer"),
        }),
        Step::Stale(turn_complete("end_turn")),
        Step::Event(delta("fresh")),
    ]
}

/// The runtime reports the same call twice: a retried start and a second end.
fn duplicate_tool_events() -> Vec<Step> {
    from_events(vec![
        tool_start("toolu-parity-dup", "Grep", r#"{"pattern":"first"}"#),
        tool_start("toolu-parity-dup", "Grep", r#"{"pattern":"second"}"#),
        tool_end("toolu-parity-dup", "Grep", "first result", false),
        tool_end("toolu-parity-dup", "Grep", "second result", true),
    ])
}

/// The turn fails while a tool is running.
fn error_turn() -> Vec<Step> {
    from_events(vec![
        delta("Starting "),
        tool_start("toolu-parity-err", "Bash", r#"{"command":"make"}"#),
        QueryEvent::Error("overloaded_error (529)".to_string()),
    ])
}

/// The worker hands over the finalized transcript, then completes the turn.
fn durable_completion() -> Vec<Step> {
    let mut assistant = Message::assistant_blocks(vec![
        ContentBlock::Text {
            text: "Here is the summary.".to_string(),
        },
        ContentBlock::ToolUse {
            id: "toolu-parity-durable".to_string(),
            name: "Read".to_string(),
            input: serde_json::json!({ "file_path": "COVEN.md" }),
        },
    ]);
    assistant.uuid = Some("msg-parity-durable".to_string());
    let result = Message::user_blocks(vec![ContentBlock::ToolResult {
        tool_use_id: "toolu-parity-durable".to_string(),
        content: ToolResultContent::Text("coven text".to_string()),
        is_error: Some(false),
    }]);
    from_events(vec![
        delta("Here is the summary."),
        QueryEvent::DurableMessage { message: assistant },
        QueryEvent::DurableMessage { message: result },
        turn_complete("end_turn"),
    ])
}

fn draw(app: &App, renderer: UiRenderer, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| match renderer {
            UiRenderer::Classic => claurst_tui::render::render_app(frame, app),
            UiRenderer::Next => claurst_tui::next::render_next(frame, app),
        })
        .expect("draw frame");
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replay `steps` for both renderers and require the same semantic state and
/// operations after every step. Returns the Classic and Next replays.
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

#[test]
fn each_renderer_reaches_the_same_semantic_state_for_every_trace() {
    // Per-trace checks run on both replays, so neither side can satisfy the
    // comparison by both being wrong in a way only one App shows.
    let (classic, next) = assert_parity("plain turn", plain_turn());
    for plain in [&classic, &next] {
        let projected = TranscriptView::project(&plain.app);
        assert_eq!(projected.live_text, "Reading the contract now.");
        let tool = &plain.last().tools["toolu-parity-1"];
        assert_eq!(tool.result.as_deref(), Some("contract text"));
        assert_eq!(tool.status, ToolStatus::Done);
        assert!(plain.last().is_streaming);
    }

    let (classic, next) = assert_parity("stale generation", stale_generation());
    for stale in [&classic, &next] {
        // Nothing a stale worker sends changes the state left at the advance.
        let at_advance = &stale.steps[2];
        for (index, step) in stale.steps[3..9].iter().enumerate() {
            assert_eq!(
                step,
                at_advance,
                "{:?}: stale step {} changed state",
                stale.renderer,
                index + 3
            );
        }
        assert!(!stale.last().tools.contains_key("toolu-parity-stale"));
        assert_eq!(
            stale.last().tools["toolu-parity-old"].status,
            ToolStatus::Running
        );
        assert_eq!(stale.app.streaming_text.as_str(), "before the clear fresh");
    }

    let (classic, next) = assert_parity("duplicate tool events", duplicate_tool_events());
    for duplicate in [&classic, &next] {
        assert_eq!(
            duplicate.app.tool_use_blocks.len(),
            1,
            "{:?}: one row per call id",
            duplicate.renderer
        );
        assert_eq!(
            duplicate.last().tools["toolu-parity-dup"],
            ToolObservation {
                name: "Grep".to_string(),
                input: r#"{"pattern":"second"}"#.to_string(),
                result: Some("second result".to_string()),
                status: ToolStatus::Error,
            }
        );
    }

    let (classic, next) = assert_parity("error", error_turn());
    for error in [&classic, &next] {
        assert!(!error.last().is_streaming);
        assert!(error
            .last()
            .messages
            .contains("Error: overloaded_error (529)"));
    }

    let (classic, next) = assert_parity("durable message and completion", durable_completion());
    for durable in [&classic, &next] {
        assert!(!durable.last().is_streaming);
        assert!(durable.last().messages.contains("msg-parity-durable"));
        let tool = &durable.last().tools["toolu-parity-durable"];
        assert_eq!(tool.result.as_deref(), Some("coven text"));
        assert_eq!(tool.status, ToolStatus::Done);
    }
}

#[test]
fn both_renderers_show_the_projected_transcript_and_live_output() {
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        let replay = Replay::new(renderer).run(plain_turn());
        let frame = draw(&replay.app, renderer, 120, 40);
        assert!(
            frame.contains("Summarize the engine contract."),
            "{renderer:?} lost the user turn:\n{frame}"
        );
        assert!(
            frame.contains("Reading the contract now."),
            "{renderer:?} lost live output:\n{frame}"
        );
    }
    let next = Replay::new(UiRenderer::Next).run(plain_turn());
    let frame = draw(&next.app, UiRenderer::Next, 120, 40);
    assert!(frame.contains(claurst_tui::next::PREVIEW_LABEL), "{frame}");
    assert!(
        frame.contains("▸ Read ENGINE-CONTRACT.md ○ done"),
        "{frame}"
    );
}

fn bash_request(id: &str) -> PermissionRequest {
    PermissionRequest::bash(
        id.to_string(),
        "Bash".to_string(),
        "Runs a shell command in the project".to_string(),
        "cargo test --workspace".to_string(),
        Some("cargo test".to_string()),
    )
}

/// A permission request opened the way the interactive loop opens it: Next
/// guards it against stray keys (#411); Classic keeps its dialog as it was.
fn opened_request(renderer: UiRenderer, id: &str) -> PermissionRequest {
    let request = bash_request(id);
    if renderer == UiRenderer::Next {
        request.guarded()
    } else {
        request
    }
}

/// Answer the open request with `codes`, pressed once it is armed, and
/// return (tool use id, chosen option, confirmed).
fn decide(app: &mut App, renderer: UiRenderer, codes: &[KeyCode]) -> (String, usize, bool) {
    let mut confirmed = false;
    for &code in codes {
        draw(app, renderer, 120, 40);
        let request = app.permission_request.as_mut().expect("pending request");
        let armed = request.quiet_since + PERMISSION_ARM_DELAY;
        confirmed = handle_permission_key_at(request, key(code), armed);
    }
    let request = app.permission_request.take().expect("pending request");
    (request.tool_use_id, request.selected_option, confirmed)
}

/// What the interactive loop would submit or decide after a key sequence.
#[derive(Debug, PartialEq, Eq)]
struct Outgoing {
    submitted: String,
    decisions: Vec<(String, usize, bool)>,
    view: TranscriptView,
}

/// Drive an App with `renderer` selected through typing, permission decisions
/// and a second draft, drawing between every key the way the interactive loop
/// does, and record the operations the loop would send.
fn drive(renderer: UiRenderer) -> Outgoing {
    let mut app = Replay::new(renderer).run(plain_turn()).app;
    for ch in "run the tests".chars() {
        draw(&app, renderer, 120, 40);
        app.handle_key_event(key(KeyCode::Char(ch)));
    }
    draw(&app, renderer, 120, 40);
    let submitted = app.take_input();

    // Keys that choose explicitly: allow once, deny, and moving to the top
    // before stepping down one, which ends on the same option from wherever
    // the request opened.
    let choices: [&[KeyCode]; 3] = [
        &[KeyCode::Char('y')],
        &[KeyCode::Esc],
        &[
            KeyCode::Up,
            KeyCode::Up,
            KeyCode::Up,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
        ],
    ];
    let mut decisions = Vec::new();
    for (index, codes) in choices.into_iter().enumerate() {
        let id = format!("toolu-parity-decide-{index}");
        app.permission_request = Some(opened_request(renderer, &id));
        decisions.push(decide(&mut app, renderer, codes));
    }

    for ch in "next draft".chars() {
        draw(&app, renderer, 120, 40);
        app.handle_key_event(key(KeyCode::Char(ch)));
    }
    draw(&app, renderer, 120, 40);
    Outgoing {
        submitted,
        decisions,
        view: TranscriptView::project(&app),
    }
}

#[test]
fn renderers_produce_identical_outgoing_operations_for_the_same_input() {
    let classic = drive(UiRenderer::Classic);
    let next = drive(UiRenderer::Next);
    assert_eq!(classic, next);
    assert_eq!(classic.submitted, "run the tests");
    let deny = bash_request("toolu-parity-options")
        .options
        .iter()
        .position(|option| option.key == 'n')
        .expect("a deny option");
    let chosen: Vec<_> = classic
        .decisions
        .iter()
        .map(|(_, option, confirmed)| (*option, *confirmed))
        .collect();
    assert_eq!(chosen, [(0, true), (deny, true), (1, true)]);
    assert_eq!(classic.view.draft, "next draft");
}

/// The intended difference (#411): Next opens a request on deny and ignores
/// keys until it is armed; Classic opens on the first option.
#[test]
fn permission_requests_open_differently_by_design() {
    let deny = bash_request("toolu-parity-options")
        .options
        .iter()
        .position(|option| option.key == 'n')
        .expect("a deny option");
    for (renderer, opened_on, early_key_applies) in [
        (UiRenderer::Classic, 0, true),
        (UiRenderer::Next, deny, false),
    ] {
        let mut app = fixture_app(renderer);
        app.permission_request = Some(opened_request(renderer, "toolu-parity-open"));
        let request = app.permission_request.as_mut().expect("pending request");
        assert_eq!(request.selected_option, opened_on, "{renderer:?}");
        let early = request.quiet_since;
        assert_eq!(
            handle_permission_key_at(request, key(KeyCode::Char('y')), early),
            early_key_applies,
            "{renderer:?}: a key pressed as the request opens"
        );
        // Enter on a request just opened, once armed, takes its focus.
        let mut fresh = opened_request(renderer, "toolu-parity-enter");
        let armed = fresh.quiet_since + PERMISSION_ARM_DELAY;
        assert!(handle_permission_key_at(
            &mut fresh,
            key(KeyCode::Enter),
            armed
        ));
        assert_eq!(fresh.selected_option, opened_on, "{renderer:?}");
    }
    // So relative keys decide differently. The sequence this comparison used
    // before #411 allows option 1 on Classic, the option above deny on Next
    // once armed, and nothing on Next pressed as the request opens.
    for (renderer, armed, decided) in [
        (UiRenderer::Classic, false, (1, true)),
        (UiRenderer::Classic, true, (1, true)),
        (UiRenderer::Next, false, (deny, false)),
        (UiRenderer::Next, true, (deny - 1, true)),
    ] {
        let mut request = opened_request(renderer, "toolu-parity-relative");
        let at = if armed {
            request.quiet_since + PERMISSION_ARM_DELAY
        } else {
            request.quiet_since
        };
        let mut confirmed = false;
        for code in [KeyCode::Down, KeyCode::Down, KeyCode::Up, KeyCode::Enter] {
            confirmed = handle_permission_key_at(&mut request, key(code), at);
        }
        assert_eq!(
            (request.selected_option, confirmed),
            decided,
            "{renderer:?}, armed: {armed}"
        );
    }
}

/// #408: while a cancel is pending, a tool start still queued from the
/// cancelled turn is recorded but does not restart the turn, so a later
/// Ctrl+C reaches the exit sequence instead of cancelling again. The worker's
/// tool result, durable messages and completion still apply, and once the
/// cancel settles no row is left running.
#[test]
fn next_backlog_tool_start_does_not_restart_a_cancelled_turn() {
    let backlog = tool_start("toolu-parity-backlog", "Bash", r#"{"command":"sleep 30"}"#);
    let cancelled = Replay::new(UiRenderer::Next).run([
        Step::Event(delta("partial answer ")),
        Step::Key(key(KeyCode::Esc)),
        Step::Event(backlog.clone()),
        Step::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
    ]);
    let at_escape = &cancelled.steps[1];
    assert!(at_escape.cancel_requested, "Escape asks the loop to cancel");
    assert_eq!(at_escape.cancellation, TurnCancellation::AwaitingRuntime);
    let at_backlog = &cancelled.steps[2];
    assert!(!at_backlog.is_streaming, "the backlog restarted the turn");
    assert_eq!(at_backlog.cancellation, TurnCancellation::AwaitingRuntime);
    assert!(
        at_backlog.tools.contains_key("toolu-parity-backlog"),
        "the backlog tool is still recorded"
    );
    let at_ctrl_c = cancelled.last();
    assert!(!at_ctrl_c.cancel_requested, "Ctrl+C cancelled again");
    assert_eq!(cancelled.app.exit_key_sequence_start, Some('c'));
    assert!(cancelled
        .app
        .notifications
        .notifications
        .iter()
        .any(|notification| notification.kind == NotificationKind::Info
            && notification.message == "Press Ctrl+C again to exit"));

    let mut assistant = Message::assistant("partial answer ");
    assistant.uuid = Some("msg-parity-backlog".to_string());
    let finished = cancelled.run([
        Step::Event(tool_end("toolu-parity-backlog", "Bash", "cancelled", true)),
        Step::Event(QueryEvent::DurableMessage { message: assistant }),
        Step::Event(turn_complete("aborted")),
        Step::Settle,
    ]);
    let tool = &finished.last().tools["toolu-parity-backlog"];
    assert_eq!(tool.result.as_deref(), Some("cancelled"));
    assert_eq!(tool.status, ToolStatus::Error);
    assert!(finished.last().messages.contains("msg-parity-backlog"));
    assert!(
        finished.app.last_turn_elapsed.is_some(),
        "the completion still applies"
    );
    assert!(!finished.last().is_streaming);
    assert_eq!(finished.last().cancellation, TurnCancellation::Idle);
    assert!(finished
        .app
        .tool_use_blocks
        .iter()
        .all(|block| block.status != ToolStatus::Running));

    // A backlog tool that never ends is interrupted when the cancel settles;
    // one that ended keeps its row.
    let unfinished = Replay::new(UiRenderer::Next).run([
        Step::Event(delta("partial answer ")),
        Step::Key(key(KeyCode::Esc)),
        Step::Event(backlog),
        Step::Event(tool_start(
            "toolu-parity-backlog-done",
            "Read",
            r#"{"file_path":"COVEN.md"}"#,
        )),
        Step::Event(tool_end(
            "toolu-parity-backlog-done",
            "Read",
            "coven text",
            false,
        )),
        // Reported finished, its result not yet delivered.
        Step::Event(tool_start(
            "toolu-parity-backlog-reported",
            "Grep",
            r#"{"pattern":"contract"}"#,
        )),
        Step::Event(QueryEvent::ToolFinished {
            tool_id: "toolu-parity-backlog-reported".to_string(),
            is_error: false,
        }),
        Step::Settle,
    ]);
    let kept: Vec<_> = unfinished
        .app
        .tool_use_blocks
        .iter()
        .map(|block| block.id.as_str())
        .collect();
    assert_eq!(
        kept,
        ["toolu-parity-backlog-done", "toolu-parity-backlog-reported"]
    );
    // None is left shown as running: the one that reported finishing is
    // shown as done until its result arrives.
    assert!(unfinished
        .app
        .tool_use_blocks
        .iter()
        .all(|block| block.status != ToolStatus::Running || block.run.finished.is_some()));
    assert!(
        unfinished
            .last()
            .observed
            .contains_key("toolu-parity-backlog"),
        "the backlog tool is still recorded"
    );
    assert!(unfinished
        .app
        .interrupted_tools
        .iter()
        .any(|block| block.id == "toolu-parity-backlog"));

    // The backlog repeats the start of the call Escape interrupted: that call
    // keeps its one interrupted row, and what it reports afterwards reaches
    // that row.
    let sleep = || {
        tool_start(
            "toolu-parity-at-escape",
            "Bash",
            r#"{"command":"sleep 30"}"#,
        )
    };
    let repeated = Replay::new(UiRenderer::Next).run([
        Step::Event(sleep()),
        Step::Key(key(KeyCode::Esc)),
        Step::Event(sleep()),
        Step::Event(QueryEvent::ToolFinished {
            tool_id: "toolu-parity-at-escape".to_string(),
            is_error: true,
        }),
        Step::Event(tool_end("toolu-parity-at-escape", "Bash", "stopped", true)),
        Step::Settle,
    ]);
    assert!(
        !repeated.steps[2].is_streaming,
        "the repeat restarted the turn"
    );
    let rows: Vec<_> = repeated
        .app
        .tool_use_blocks
        .iter()
        .map(|block| ("live", block.id.as_str()))
        .chain(
            repeated
                .app
                .interrupted_tools
                .iter()
                .map(|block| ("interrupted", block.id.as_str())),
        )
        .collect();
    assert_eq!(rows, [("interrupted", "toolu-parity-at-escape")]);
    let row = &repeated.app.interrupted_tools[0];
    assert_eq!(row.output_preview.as_deref(), Some("stopped"));
    assert_eq!(row.status, ToolStatus::Error);
    assert!(row.run.finished.is_some_and(|(_, is_error)| is_error));
}

#[test]
fn next_draws_permission_requests_itself_unless_another_surface_is_open() {
    let mut app = Replay::new(UiRenderer::Next).run(plain_turn()).app;
    app.permission_request = Some(bash_request("toolu-parity-3"));
    // #411: Next owns the approval surface.
    assert!(!claurst_tui::next::delegates_to_classic(&app));
    let next = draw(&app, UiRenderer::Next, 120, 40);
    assert!(next.contains("cargo test --workspace"), "{next}");
    let request = app.permission_request.as_ref().expect("request");
    for option in &request.options {
        assert!(
            next.contains(&option.label),
            "next lacks {:?}:\n{next}",
            option.label
        );
    }
    // With another input-owning surface open, Classic's precedence applies.
    app.help_overlay.visible = true;
    assert!(claurst_tui::next::delegates_to_classic(&app));
    assert_eq!(
        draw(&app, UiRenderer::Next, 120, 40),
        draw(&app, UiRenderer::Classic, 120, 40)
    );
}

/// Opens one input-owning surface on an App.
type OpenSurface = fn(&mut App);

#[test]
fn next_hands_every_input_owning_surface_to_classic() {
    let cases: Vec<(&str, OpenSurface)> = vec![
        ("help overlay", |app| app.help_overlay.visible = true),
        ("familiar switcher", |app| app.familiar_switcher_open = true),
        ("effort picker", |app| app.effort_picker.visible = true),
        ("response reader", |app| app.response_reader.visible = true),
    ];
    for (name, open) in cases {
        let mut app = Replay::new(UiRenderer::Next).run(plain_turn()).app;
        assert!(
            !claurst_tui::next::delegates_to_classic(&app),
            "{name}: plain shell"
        );
        open(&mut app);
        assert!(claurst_tui::next::delegates_to_classic(&app), "{name}");
        assert_eq!(
            draw(&app, UiRenderer::Next, 120, 40),
            draw(&app, UiRenderer::Classic, 120, 40),
            "{name}"
        );
    }
}

#[test]
fn next_replaces_classic_mouse_geometry_with_its_own_transcript_target() {
    let app = Replay::new(UiRenderer::Next).run(plain_turn()).app;
    draw(&app, UiRenderer::Classic, 120, 40);
    assert!(
        app.last_input_area.get().area() > 0,
        "classic records geometry"
    );
    let classic_selectable = app.last_selectable_area.get();
    draw(&app, UiRenderer::Next, 120, 40);
    // No per-message hit-test survives from Classic's layout.
    assert!(app.message_row_map.borrow().is_empty());
    assert!(app.thinking_row_map.borrow().is_empty());
    assert_eq!(app.last_input_area.get().area(), 0);
    // Selection and copy target the Next transcript pane only (#410).
    let selectable = app.last_selectable_area.get();
    assert!(selectable.area() > 0);
    assert_ne!(selectable, classic_selectable);
    assert_eq!(app.last_msg_area.get(), selectable);
    assert!(app
        .last_row_text
        .borrow()
        .keys()
        .all(|row| *row >= selectable.y && *row < selectable.y + selectable.height));
}

#[test]
fn next_keeps_newest_output_visible_at_small_sizes_without_panicking() {
    let mut long = plain_turn();
    long.push(Step::Event(delta(
        &("\n".to_string() + &"wide 漢字 text ".repeat(40) + "\nnewest line"),
    )));
    let app = Replay::new(UiRenderer::Next).run(long).app;
    for (width, height) in [(40, 12), (80, 24), (1, 4)] {
        let frame = draw(&app, UiRenderer::Next, width, height);
        if width >= 40 {
            assert!(frame.contains("newest line"), "{width}x{height}:\n{frame}");
        }
    }
}

/// An error notice is not handed to Classic: Classic's modal draws its text
/// raw (#417), so Next draws its own inert panel. Esc still dismisses it
/// through the shared key handling.
#[test]
fn next_draws_error_notices_itself_and_esc_still_dismisses_them() {
    let mut app = Replay::new(UiRenderer::Next).run(plain_turn()).app;
    app.push_notification(
        NotificationKind::Error,
        "synthetic failure".to_string(),
        None,
    );
    assert!(!claurst_tui::next::delegates_to_classic(&app));
    let next = draw(&app, UiRenderer::Next, 120, 40);
    assert_ne!(next, draw(&app, UiRenderer::Classic, 120, 40));
    app.handle_key_event(key(KeyCode::Esc));
    assert!(!app.notifications.current_is_error(), "Esc dismissed it");
}
