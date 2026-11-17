//! Duplicate, out-of-order and missing runtime evidence (#408).
//!
//! #408 acceptance: "Duplicate observations do not duplicate commands or
//! commits. Out-of-order or missing authoritative evidence produces
//! reconciliation, not invented success." Late events from an earlier
//! session generation are covered in the CLI, which assigns and advances the
//! generation (`crates/cli/src/reconcile_tests.rs`).
//!
//! Each test drives an App per renderer, selected the way the interactive
//! loop selects it, and feeds it the way the loop does: through
//! `SessionScope::apply` for the current generation, drawing after every
//! event. Each asserts the outcome itself, in both renderers, and that both
//! reach the same outcome.
//!
//! What the App hands back to the loop is checked after every event: the
//! effects a step returns (the bell), a cancel request, an automatic
//! submission and the queued messages. Events never reach the saved session
//! directly: the CLI saves the worker's own transcript when the worker is
//! joined. The App's transcript reaches a saved session only when the loop
//! adopts an in-App rewind, so these tests check the App's transcript too.

use std::time::{Duration, Instant};

use claurst_api::streaming::ContentDelta;
use claurst_api::AnthropicStreamEvent;
use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::types::{ContentBlock, Message, ToolResultContent, UsageInfo};
use claurst_query::QueryEvent;
use claurst_tui::app::App;
use claurst_tui::application::view::{ToolActivityState, TranscriptView};
use claurst_tui::application::{
    PresentationEffect, ScopedQueryEvent, SessionScope, TurnCancellation, UiRenderer,
};
use claurst_tui::NotificationKind;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

const RENDERERS: [UiRenderer; 2] = [UiRenderer::Classic, UiRenderer::Next];

/// How long ago the turn under test was submitted: past the shortest turn
/// that gets a completion toast and bell, so a completion is announced.
const TURN_AGE: Duration = Duration::from_secs(30);

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

fn tool_finished(id: &str) -> QueryEvent {
    QueryEvent::ToolFinished {
        tool_id: id.to_string(),
        is_error: false,
    }
}

fn end_turn() -> QueryEvent {
    QueryEvent::TurnComplete {
        turn: 1,
        stop_reason: "end_turn".to_string(),
        usage: Some(UsageInfo {
            input_tokens: 1000,
            output_tokens: 200,
            ..Default::default()
        }),
    }
}

fn durable_answer(uuid: &str, text: &str, tool: Option<(&str, &str)>) -> QueryEvent {
    let mut blocks = vec![ContentBlock::Text {
        text: text.to_string(),
    }];
    if let Some((id, name)) = tool {
        blocks.push(ContentBlock::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input: serde_json::json!({ "command": "cargo test" }),
        });
    }
    let mut message = Message::assistant_blocks(blocks);
    message.uuid = Some(uuid.to_string());
    QueryEvent::DurableMessage { message }
}

fn durable_result(id: &str, text: &str) -> QueryEvent {
    QueryEvent::DurableMessage {
        message: Message::user_blocks(vec![ContentBlock::ToolResult {
            tool_use_id: id.to_string(),
            content: ToolResultContent::Text(text.to_string()),
            is_error: Some(false),
        }]),
    }
}

/// Everything an event can change in the workspace, or hand to the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Workspace {
    /// The App's transcript, serialized.
    messages: String,
    /// Transcript entries, live output, tool rows with their states, status
    /// and draft, as both renderers project them.
    view: TranscriptView,
    notifications: Vec<(NotificationKind, String)>,
    /// Classic's "done" marker after a completed turn: its elapsed label.
    done_marker: Option<String>,
    /// Per user turn: the recorded duration and whether it was interrupted.
    turns: Vec<(Option<String>, bool)>,
    context_used_tokens: u64,
    cancellation: TurnCancellation,
    auto_submit: bool,
    queued: usize,
}

/// An App mid-turn, fed and drawn the way the interactive loop does it.
struct Turn {
    renderer: UiRenderer,
    app: App,
    scope: SessionScope,
    /// Every effect a step returned, in order.
    effects: Vec<PresentationEffect>,
    cancel_requests: usize,
}

impl Turn {
    /// A prompt submitted `TURN_AGE` ago with the bell and completion toast
    /// on and the production threshold for announcing a completion.
    fn started(renderer: UiRenderer) -> Self {
        let config = Config {
            model: Some("claude-baseline-model".to_string()),
            provider: Some("anthropic".to_string()),
            ..Config::default()
        };
        let mut app = App::new(config, CostTracker::new());
        app.has_credentials = true;
        app.select_renderer(renderer);
        assert_eq!(
            app.renderer, renderer,
            "the test runs the renderer under test"
        );
        app.bell_on_complete = true;
        app.completion_toast_enabled = true;
        app.push_message(Message::user("Run the tests.".to_string()));
        app.turn_start = Some(
            Instant::now()
                .checked_sub(TURN_AGE)
                .expect("the host has been up for longer than TURN_AGE"),
        );
        // As the loop does when it starts the worker.
        app.is_streaming = true;
        Self {
            renderer,
            app,
            scope: SessionScope::default(),
            effects: Vec::new(),
            cancel_requests: 0,
        }
    }

    /// Apply one event from the current generation's worker, draw, and hand
    /// any cancel request on. Returns the effects this step asked for.
    fn apply(&mut self, event: QueryEvent) -> Vec<PresentationEffect> {
        let effects = self.scope.apply(
            &mut self.app,
            ScopedQueryEvent {
                generation: self.scope.current(),
                event,
            },
        );
        draw(&self.app, self.renderer);
        if self.app.take_cancellation_request() {
            self.cancel_requests += 1;
        }
        self.effects.extend(effects.iter().copied());
        effects
    }

    fn run(&mut self, events: impl IntoIterator<Item = QueryEvent>) {
        for event in events {
            self.apply(event);
        }
    }

    fn workspace(&self) -> Workspace {
        let app = &self.app;
        Workspace {
            messages: serde_json::to_string(&*app.messages).expect("messages serialize"),
            view: TranscriptView::project(app),
            notifications: app
                .notifications
                .notifications
                .iter()
                .map(|notification| (notification.kind.clone(), notification.message.clone()))
                .collect(),
            done_marker: app.last_turn_verb.and(app.last_turn_elapsed.clone()),
            turns: app
                .turn_metadata
                .iter()
                .map(|turn| (turn.duration.clone(), turn.interrupted))
                .collect(),
            context_used_tokens: app.context_used_tokens,
            cancellation: app.cancellation,
            auto_submit: app.pending_auto_submit,
            queued: app.queued_messages.len(),
        }
    }

    /// Nothing went back to the loop: no cancel, no submission, no queue.
    fn assert_sent_nothing(&self, case: &str) {
        let workspace = self.workspace();
        assert_eq!(
            self.cancel_requests, 0,
            "{case} {:?}: cancel",
            self.renderer
        );
        assert!(
            !workspace.auto_submit,
            "{case} {:?}: submission",
            self.renderer
        );
        assert_eq!(workspace.queued, 0, "{case} {:?}: queue", self.renderer);
    }

    fn tool_state(&self, id: &str) -> Option<ToolActivityState> {
        TranscriptView::project(&self.app)
            .tools
            .iter()
            .find(|tool| tool.id == id)
            .map(|tool| tool.state)
    }

    fn warnings(&self) -> Vec<String> {
        self.app
            .notifications
            .notifications
            .iter()
            .filter(|notification| notification.kind == NotificationKind::Warning)
            .map(|notification| notification.message.clone())
            .collect()
    }

    fn successes(&self) -> usize {
        self.app
            .notifications
            .notifications
            .iter()
            .filter(|notification| notification.kind == NotificationKind::Success)
            .count()
    }

    /// The turn is not reported as having succeeded: no done marker, no
    /// "Turn done" toast, no bell, and its turn is recorded as interrupted.
    fn assert_not_reported_done(&self, case: &str) {
        let renderer = self.renderer;
        assert_eq!(
            self.app.last_turn_verb, None,
            "{case} {renderer:?}: done marker"
        );
        assert_eq!(self.successes(), 0, "{case} {renderer:?}: completion toast");
        assert!(
            self.effects.is_empty(),
            "{case} {renderer:?}: {:?}",
            self.effects
        );
        let turn = self.app.turn_metadata.last().expect("the submitted turn");
        assert!(
            turn.interrupted,
            "{case} {renderer:?}: turn recorded as complete"
        );
    }
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

fn texts(app: &App) -> Vec<String> {
    app.messages.iter().map(Message::get_all_text).collect()
}

fn draw(app: &App, renderer: UiRenderer) {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("test terminal");
    terminal
        .draw(|frame| match renderer {
            UiRenderer::Classic => claurst_tui::render::render_app(frame, app),
            UiRenderer::Next => claurst_tui::next::render_next(frame, app),
        })
        .expect("draw frame");
}

/// Run `case` for each renderer and require both to end in the same
/// workspace.
fn in_both_renderers(case: impl Fn(UiRenderer) -> Turn) {
    let [classic, next] = RENDERERS.map(case);
    assert_eq!(
        classic.workspace(),
        next.workspace(),
        "the renderers reached different workspaces"
    );
}

/// A repeated start, end or finish of one call leaves one row with its one
/// result, and a start delivered again after the end does not reopen it.
#[test]
fn duplicate_tool_observations_keep_one_row_and_its_result() {
    in_both_renderers(|renderer| {
        let mut turn = Turn::started(renderer);
        let start = || tool_start("toolu-dup", "Bash", r#"{"command":"cargo test"}"#);
        turn.run([start(), start(), tool_finished("toolu-dup")]);
        turn.apply(tool_end("toolu-dup", "Bash", "ok", false));
        let ended = turn.workspace();
        for duplicate in [
            tool_end("toolu-dup", "Bash", "ok", false),
            tool_finished("toolu-dup"),
            start(),
        ] {
            let name = format!("{duplicate:?}");
            assert!(turn.apply(duplicate).is_empty(), "{renderer:?}: {name}");
            assert_eq!(
                turn.workspace(),
                ended,
                "{renderer:?}: {name} changed the workspace"
            );
        }
        let view = TranscriptView::project(&turn.app);
        assert_eq!(view.tools.len(), 1, "{renderer:?}: one row per call");
        assert_eq!(view.tools[0].state, ToolActivityState::Done, "{renderer:?}");
        assert_eq!(view.tools[0].output.as_deref(), Some("ok"), "{renderer:?}");
        turn.assert_sent_nothing("duplicate tool events");

        // Every call has its result, so the turn completes as done, once.
        turn.apply(end_turn());
        assert_eq!(turn.effects, [PresentationEffect::RingBell], "{renderer:?}");
        assert!(
            turn.app.last_turn_verb.is_some(),
            "{renderer:?}: done marker"
        );

        // The runtime starts each call once, so a start with that id in a
        // later turn is a new call, as before #408.
        turn.app.push_message(Message::user("Again.".to_string()));
        turn.app.is_streaming = true;
        turn.apply(start());
        assert_eq!(
            turn.tool_state("toolu-dup"),
            Some(ToolActivityState::Running),
            "{renderer:?}: a later turn's call"
        );
        turn
    });
}

/// A finalized message or tool result delivered twice is one transcript entry
/// and one result.
#[test]
fn a_duplicate_durable_message_is_one_transcript_entry() {
    in_both_renderers(|renderer| {
        let mut turn = Turn::started(renderer);
        turn.apply(delta("Running them."));
        let answer = || durable_answer("msg-dup", "Running them.", Some(("toolu-durable", "Bash")));
        turn.run([answer(), durable_result("toolu-durable", "42 passed")]);
        let delivered = turn.workspace();
        for duplicate in [answer(), durable_result("toolu-durable", "42 passed")] {
            assert!(turn.apply(duplicate).is_empty(), "{renderer:?}");
            assert_eq!(
                turn.workspace(),
                delivered,
                "{renderer:?}: a duplicate changed it"
            );
        }
        let assistant = turn
            .app
            .messages
            .iter()
            .filter(|message| message.uuid.as_deref() == Some("msg-dup"))
            .count();
        assert_eq!(
            assistant, 1,
            "{renderer:?}: the answer is in the transcript once"
        );
        assert_eq!(
            turn.app.messages.len(),
            2,
            "{renderer:?}: prompt and answer only"
        );
        assert_eq!(
            turn.tool_state("toolu-durable"),
            Some(ToolActivityState::Done),
            "{renderer:?}"
        );
        turn.assert_sent_nothing("duplicate durable message");
        turn
    });
}

/// A turn completion delivered twice rings and toasts once, keeps the
/// recorded duration, counts the context once and commits no second message.
#[test]
fn a_duplicate_turn_complete_commits_the_turn_once() {
    // The production threshold, and none at all: a duplicate must not ring
    // again even when every completion would be announced.
    for shortest_announced in [8, 0] {
        in_both_renderers(|renderer| {
            let mut turn = Turn::started(renderer);
            turn.app.completion_toast_min_secs = shortest_announced;
            turn.run([
                delta("All tests pass."),
                durable_answer("msg-complete", "All tests pass.", None),
            ]);
            assert_eq!(
                turn.apply(end_turn()),
                [PresentationEffect::RingBell],
                "{renderer:?}: the first completion rings"
            );
            let completed = turn.workspace();
            assert!(completed.done_marker.is_some(), "{renderer:?}: done marker");
            assert_eq!(turn.successes(), 1, "{renderer:?}: one completion toast");
            assert_eq!(completed.context_used_tokens, 1200, "{renderer:?}");

            let again = turn.apply(end_turn());
            assert!(
                again.is_empty(),
                "{renderer:?}: the duplicate rang: {again:?}"
            );
            assert_eq!(
                turn.workspace(),
                completed,
                "{renderer:?}, threshold {shortest_announced}s: the duplicate changed it"
            );
            turn.assert_sent_nothing("duplicate completion");
            turn
        });
    }
}

/// An end for a call the App has not seen start is not applied, so the call
/// is not shown done on its late start, and a turn that then completes is
/// not reported done and says the call had no result. Its finalized result,
/// when it arrives, is authoritative and reconciles both.
#[test]
fn an_end_before_its_start_does_not_show_the_call_done() {
    let call = "toolu-early-end";
    let start = || tool_start(call, "Bash", r#"{"command":"cargo test"}"#);
    in_both_renderers(|renderer| {
        let mut turn = Turn::started(renderer);
        turn.run([tool_finished(call), tool_end(call, "Bash", "ok", false)]);
        assert_eq!(
            turn.tool_state(call),
            None,
            "{renderer:?}: an end created a row"
        );
        turn.apply(start());
        assert_eq!(
            turn.tool_state(call),
            Some(ToolActivityState::Running),
            "{renderer:?}: the late start shows the call done"
        );
        turn.apply(end_turn());
        // The completion drops the row, as before, and says the call had no
        // result.
        assert_eq!(turn.tool_state(call), None, "{renderer:?}");
        assert_eq!(
            turn.warnings(),
            ["Turn ended with no result from Bash"],
            "{renderer:?}"
        );
        turn.assert_not_reported_done("end before start");
        turn.assert_sent_nothing("end before start");
        turn
    });
    in_both_renderers(|renderer| {
        let mut turn = Turn::started(renderer);
        turn.run([
            tool_end(call, "Bash", "ok", false),
            start(),
            durable_result(call, "ok"),
        ]);
        assert_eq!(
            turn.tool_state(call),
            Some(ToolActivityState::Done),
            "{renderer:?}: the finalized result applies"
        );
        turn.apply(end_turn());
        assert_eq!(turn.effects, [PresentationEffect::RingBell], "{renderer:?}");
        assert!(
            turn.app.last_turn_verb.is_some(),
            "{renderer:?}: done marker"
        );
        turn
    });
}

/// A completion while a call has no result does not report the turn done,
/// and says which call had none. A call that did report keeps its result.
#[test]
fn a_turn_that_completes_without_a_tool_result_is_not_reported_done() {
    in_both_renderers(|renderer| {
        let mut turn = Turn::started(renderer);
        turn.run([
            tool_start("toolu-reported", "Read", r#"{"file_path":"COVEN.md"}"#),
            tool_end("toolu-reported", "Read", "coven text", false),
            tool_start("toolu-missing", "Bash", r#"{"command":"cargo test"}"#),
            tool_start("toolu-missing-2", "Bash", r#"{"command":"cargo doc"}"#),
            tool_start("toolu-odd", "Odd\u{1b}[2J", "{}"),
            delta("Done."),
            end_turn(),
        ]);
        assert!(!turn.app.is_streaming, "{renderer:?}");
        // Their rows are dropped, as before; the warning names each tool
        // once, terminal-safe.
        assert_eq!(turn.tool_state("toolu-missing"), None, "{renderer:?}");
        assert_eq!(
            turn.warnings(),
            ["Turn ended with no result from Bash, Odd^[[2J"],
            "{renderer:?}"
        );
        assert_eq!(
            turn.tool_state("toolu-reported"),
            Some(ToolActivityState::Done),
            "{renderer:?}"
        );
        turn.assert_not_reported_done("completion without a result");
        // With the unresolved row gone, a duplicate completion must not
        // report the turn done after all.
        let ended = turn.workspace();
        assert!(turn.apply(end_turn()).is_empty(), "{renderer:?}");
        assert_eq!(
            turn.workspace(),
            ended,
            "{renderer:?}: the duplicate changed it"
        );
        turn.assert_not_reported_done("completion without a result, twice");
        turn.assert_sent_nothing("completion without a result");
        turn
    });
}

/// A provider error after a partial answer: the query loop finalizes the
/// partial answer as a durable message and returns the error to the loop,
/// which ends the turn at the join. No completion arrives, so nothing reports
/// the turn done. (`QueryEvent::Error` is not sent by the runtime.)
#[test]
fn an_error_after_a_partial_answer_is_not_reported_done() {
    in_both_renderers(|renderer| {
        let mut turn = Turn::started(renderer);
        turn.run([
            delta("The first half of an ans"),
            durable_answer("msg-partial", "The first half of an ans", None),
        ]);
        let app = &turn.app;
        assert_eq!(
            texts(app),
            ["Run the tests.", "The first half of an ans"],
            "{renderer:?}: the partial answer is kept once, as finalized"
        );
        assert!(app.streaming_text.is_empty(), "{renderer:?}");
        assert_eq!(app.last_turn_verb, None, "{renderer:?}: done marker");
        assert_eq!(turn.successes(), 0, "{renderer:?}: completion toast");
        assert!(turn.effects.is_empty(), "{renderer:?}: {:?}", turn.effects);
        let record = app.turn_metadata.last().expect("the submitted turn");
        assert_eq!(record.duration, None, "{renderer:?}: recorded complete");
        turn.assert_sent_nothing("error after a partial answer");
        turn
    });
}

/// A turn after an in-App rewind completes as its own turn, once: the
/// records of the turns the rewind removed do not make its completion look
/// like a duplicate.
#[test]
fn a_turn_after_a_rewind_completes_once() {
    in_both_renderers(|renderer| {
        let mut turn = Turn::started(renderer);
        turn.app.completion_toast_min_secs = 0;
        turn.run([
            delta("All tests pass."),
            durable_answer("msg-tests", "All tests pass.", None),
            end_turn(),
        ]);
        // The second turn ends in an error: a partial answer, a call that
        // never got its result, and no completion. The loop stops streaming
        // at the join and leaves the row.
        turn.app
            .push_message(Message::user("And the docs?".to_string()));
        turn.app.is_streaming = true;
        turn.run([
            delta("Checking the docs."),
            tool_start("toolu-docs", "Bash", r#"{"command":"cargo doc"}"#),
            durable_answer("msg-docs", "Checking the docs.", None),
        ]);
        turn.app.is_streaming = false;
        assert_eq!(turn.effects.len(), 1, "{renderer:?}");

        // Rewind to before "And the docs?" through the overlay.
        assert!(turn.app.intercept_slash_command_with_args("rewind", ""));
        press(&mut turn.app, KeyCode::Up);
        press(&mut turn.app, KeyCode::Enter);
        press(&mut turn.app, KeyCode::Char('y'));
        assert_eq!(
            texts(&turn.app),
            ["Run the tests.", "All tests pass."],
            "{renderer:?}"
        );
        assert_eq!(
            turn.app.turn_metadata.len(),
            1,
            "{renderer:?}: each kept turn keeps its own record"
        );

        turn.app
            .push_message(Message::user("Check the docs again.".to_string()));
        turn.app.is_streaming = true;
        turn.run([
            delta("Docs pass now."),
            durable_answer("msg-docs-again", "Docs pass now.", None),
            end_turn(),
        ]);
        assert_eq!(
            turn.effects,
            [PresentationEffect::RingBell; 2],
            "{renderer:?}: the turn after the rewind rings"
        );
        assert!(
            turn.app.last_turn_verb.is_some(),
            "{renderer:?}: done marker"
        );
        assert!(
            turn.warnings().is_empty(),
            "{renderer:?}: the removed turn's call is not this turn's"
        );
        assert_eq!(turn.tool_state("toolu-docs"), None, "{renderer:?}");
        assert_eq!(turn.app.context_used_tokens, 2400, "{renderer:?}");
        let records: Vec<bool> = turn
            .app
            .turn_metadata
            .iter()
            .map(|record| record.duration.is_some() && !record.interrupted)
            .collect();
        assert_eq!(records, [true, true], "{renderer:?}");
        turn.assert_sent_nothing("turn after a rewind");
        turn
    });
}
