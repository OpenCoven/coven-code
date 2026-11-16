//! #408: "Late events for a previous session/view generation cannot update a
//! new workspace."
//!
//! The interactive loop tags each worker's events with the generation it was
//! spawned in (`forward_scoped_events`), and admits only the current
//! generation to the App (`SessionScope::apply` in its drain). Replacing the
//! conversation starts a new generation: `/clear` through
//! `replace_conversation`, an in-App rewind through `adopt_in_app_rewind`.
//! These tests put those real pieces together around a worker that is still
//! delivering when the conversation is replaced, and check that nothing it
//! sends afterwards reaches the new workspace in either renderer: not its
//! transcript, tool rows, status, streaming state, notifications or turn
//! records, and no effect for the loop.
//!
//! In the loop neither replacement overlaps a running worker: `/clear` runs
//! only once no worker is running (input is queued while one is), an open
//! rewind holds every turn the loop would start, and a rewind is adopted
//! only after a running turn's join
//! (`a_rewind_holds_new_turns_and_waits_for_the_running_one`). These tests
//! build the overlap anyway, so the generation filter, the second guard, is
//! checked on its own.

use super::*;
use claurst_core::types::{ContentBlock, Message, ToolResultContent, UsageInfo};
use claurst_query::QueryEvent;
use claurst_tui::application::view::TranscriptView;
use claurst_tui::application::{
    PresentationEffect, ScopedQueryEvent, SessionGeneration, SessionScope, UiRenderer,
};

fn delta(text: &str) -> QueryEvent {
    QueryEvent::Stream(claurst_api::AnthropicStreamEvent::ContentBlockDelta {
        index: 0,
        delta: claurst_api::streaming::ContentDelta::TextDelta {
            text: text.to_string(),
        },
    })
}

/// One of every observation a worker can send, for a call `id`.
fn every_kind_of_event(id: &str) -> Vec<QueryEvent> {
    let mut answer = Message::assistant_blocks(vec![
        ContentBlock::Text {
            text: "late answer".to_string(),
        },
        ContentBlock::ToolUse {
            id: id.to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({ "command": "rm -rf target" }),
        },
    ]);
    answer.uuid = Some("msg-late".to_string());
    vec![
        delta("late text"),
        QueryEvent::Stream(claurst_api::AnthropicStreamEvent::MessageStop),
        QueryEvent::ToolStart {
            tool_name: "Bash".to_string(),
            tool_id: id.to_string(),
            input_json: r#"{"command":"rm -rf target"}"#.to_string(),
        },
        QueryEvent::ToolQueued {
            tool_id: id.to_string(),
        },
        QueryEvent::ToolRunning {
            tool_id: id.to_string(),
        },
        QueryEvent::ToolFinished {
            tool_id: id.to_string(),
            is_error: true,
        },
        QueryEvent::ToolEnd {
            tool_name: "Bash".to_string(),
            tool_id: id.to_string(),
            result: "late result".to_string(),
            is_error: true,
        },
        QueryEvent::Status("late status".to_string()),
        QueryEvent::TokenWarning {
            state: claurst_query::compact::TokenWarningState::Critical,
            pct_used: 0.97,
        },
        QueryEvent::DurableMessage { message: answer },
        QueryEvent::DurableMessage {
            message: Message::user_blocks(vec![ContentBlock::ToolResult {
                tool_use_id: id.to_string(),
                content: ToolResultContent::Text("late result".to_string()),
                is_error: Some(true),
            }]),
        },
        QueryEvent::Error("late failure".to_string()),
        QueryEvent::TurnComplete {
            turn: 1,
            stop_reason: "end_turn".to_string(),
            usage: Some(UsageInfo {
                input_tokens: 5000,
                output_tokens: 500,
                ..Default::default()
            }),
        },
    ]
}

/// Everything an event can change in the workspace, or hand to the loop.
#[derive(Debug, PartialEq, Eq)]
struct Workspace {
    messages: String,
    view: TranscriptView,
    interrupted: Vec<String>,
    notifications: Vec<(claurst_tui::NotificationKind, String)>,
    done_marker: Option<String>,
    turns: Vec<(Option<String>, bool)>,
    context_used_tokens: u64,
    cancellation: claurst_tui::application::TurnCancellation,
    auto_submit: bool,
}

fn workspace(app: &claurst_tui::App) -> Workspace {
    Workspace {
        messages: serde_json::to_string(&*app.messages).expect("messages serialize"),
        view: TranscriptView::project(app),
        interrupted: app
            .interrupted_tools
            .iter()
            .map(|block| block.id.clone())
            .collect(),
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
    }
}

fn texts(messages: &[Message]) -> Vec<String> {
    messages.iter().map(Message::get_all_text).collect()
}

fn press(app: &mut claurst_tui::App, code: crossterm::event::KeyCode) {
    app.handle_key_event(crossterm::event::KeyEvent::new(
        code,
        crossterm::event::KeyModifiers::NONE,
    ));
}

/// What the interactive loop holds for one session: the App, the
/// conversation it sends and saves, the session, the generation and the
/// bounded queue its workers deliver into.
struct Interactive {
    app: claurst_tui::App,
    messages: Vec<Message>,
    session: claurst_core::history::ConversationSession,
    scope: SessionScope,
    events: tokio::sync::mpsc::Sender<ScopedQueryEvent>,
    queue: tokio::sync::mpsc::Receiver<ScopedQueryEvent>,
}

impl Interactive {
    /// A session resumed with `history`, set up as the loop sets it up.
    fn resumed(renderer: UiRenderer, history: Vec<Message>) -> Self {
        let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
        app.select_renderer(renderer);
        app.bell_on_complete = true;
        app.completion_toast_min_secs = 0;
        let mut session = claurst_core::history::ConversationSession::new("model".into());
        session.messages = history.clone();
        app.replace_messages(history.clone());
        app.enter_session_scope(&session.id);
        let (events, queue) =
            tokio::sync::mpsc::channel(claurst_tui::application::QUERY_EVENT_QUEUE_CAPACITY);
        Self {
            app,
            messages: history,
            session,
            scope: SessionScope::default(),
            events,
            queue,
        }
    }

    /// Submit a prompt the way the loop does, and start a worker for it in
    /// the current generation. The worker forwards what the test sends on
    /// the returned sender, through the real forwarder, until it is dropped.
    fn submit(
        &mut self,
        prompt: &str,
    ) -> (
        tokio::sync::mpsc::UnboundedSender<QueryEvent>,
        tokio::task::JoinHandle<()>,
    ) {
        let message = Message::user(prompt.to_string());
        self.messages.push(message.clone());
        self.app.push_message(message);
        self.session.messages = self.messages.clone();
        self.app.is_streaming = true;
        self.app.streaming_text.clear();
        let (turn_tx, turn_rx) = tokio::sync::mpsc::unbounded_channel();
        let worker = tokio::spawn(forward_scoped_events(
            turn_rx,
            self.events.clone(),
            self.scope.current(),
        ));
        (turn_tx, worker)
    }

    /// Send `events` from a worker and wait until the queue holds them all.
    async fn deliver(
        &mut self,
        worker: &tokio::sync::mpsc::UnboundedSender<QueryEvent>,
        events: Vec<QueryEvent>,
    ) {
        let target = self.queue.len() + events.len();
        for event in events {
            worker.send(event).expect("worker is running");
        }
        let queued = async {
            while self.queue.len() < target {
                tokio::task::yield_now().await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), queued)
            .await
            .expect("the forwarder queues every event it is sent");
    }

    /// The loop's drain: everything queued, through the generation filter.
    /// Returns how many events were taken and the effects they asked for.
    fn drain(&mut self) -> (usize, Vec<PresentationEffect>) {
        let mut taken = 0;
        let mut effects = Vec::new();
        while let Ok(scoped) = self.queue.try_recv() {
            taken += 1;
            effects.extend(self.scope.apply(&mut self.app, scoped));
        }
        (taken, effects)
    }
}

/// How the conversation is replaced while the old worker still delivers.
#[derive(Debug, Clone, Copy)]
enum Replacement {
    /// `/clear`: the App's own clear, then the CLI's
    /// `CommandResult::ClearConversation` arm.
    Clear,
    /// The rewind overlay, opened while no turn ran, confirmed once the App
    /// stopped streaming; then the loop's adoption, which runs after every
    /// key, as if the turn had been joined.
    Rewind,
}

fn history() -> Vec<Message> {
    vec![
        Message::user("First question.".to_string()),
        Message::assistant("First answer.".to_string()),
        Message::user("Second question.".to_string()),
        Message::assistant("Second answer.".to_string()),
    ]
}

#[tokio::test]
async fn a_replaced_conversation_takes_nothing_from_its_old_worker() {
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        for replacement in [Replacement::Clear, Replacement::Rewind] {
            replace_while_the_old_worker_delivers(renderer, replacement).await;
        }
    }
}

async fn replace_while_the_old_worker_delivers(renderer: UiRenderer, replacement: Replacement) {
    let case = format!("{renderer:?}, {replacement:?}");
    let mut cli = Interactive::resumed(renderer, history());
    if let Replacement::Rewind = replacement {
        // Opened while no turn runs, on "Second question.".
        assert!(cli.app.intercept_slash_command_with_args("rewind", ""));
        press(&mut cli.app, crossterm::event::KeyCode::Up);
    }

    let (old, old_worker) = cli.submit("Third question.");
    let old_generation: SessionGeneration = cli.scope.current();
    let before = match replacement {
        // `/clear` lands mid-turn, with a call running.
        Replacement::Clear => vec![
            delta("partial "),
            QueryEvent::ToolStart {
                tool_name: "Bash".to_string(),
                tool_id: "toolu-old-running".to_string(),
                input_json: r#"{"command":"cargo test"}"#.to_string(),
            },
        ],
        // The rewind is adopted only once the App has stopped streaming.
        Replacement::Rewind => vec![
            delta("third answer"),
            QueryEvent::TurnComplete {
                turn: 1,
                stop_reason: "end_turn".to_string(),
                usage: None,
            },
        ],
    };
    let expected = before.len();
    cli.deliver(&old, before).await;
    let (taken, _) = cli.drain();
    assert_eq!(taken, expected, "{case}: the turn reached the App before");

    match replacement {
        Replacement::Clear => {
            assert!(!cli.app.tool_use_blocks.is_empty(), "{case}");
            assert!(cli.app.intercept_slash_command_with_args("clear", ""));
            replace_conversation(
                &mut cli.scope,
                Vec::new(),
                &mut cli.messages,
                &mut cli.app,
                &mut cli.session,
            );
            cli.app.status_message = Some("Conversation cleared.".to_string());
        }
        Replacement::Rewind => {
            assert!(!cli.app.is_streaming, "{case}");
            press(&mut cli.app, crossterm::event::KeyCode::Enter);
            press(&mut cli.app, crossterm::event::KeyCode::Char('y'));
            adopt_in_app_rewind(
                &mut cli.scope,
                &cli.app,
                false,
                &mut cli.messages,
                &mut cli.session,
            );
        }
    }
    assert!(
        !cli.scope.admits(old_generation),
        "{case}: the replacement started a new generation"
    );
    let replaced = workspace(&cli.app);
    match replacement {
        Replacement::Clear => {
            assert!(cli.app.messages.is_empty(), "{case}");
            assert!(cli.app.tool_use_blocks.is_empty(), "{case}");
        }
        Replacement::Rewind => {
            let kept = ["First question.", "First answer."];
            assert_eq!(texts(&cli.app.messages), kept, "{case}");
            assert_eq!(texts(&cli.messages), kept, "{case}: adopted");
        }
    }

    // Everything the old worker still sends arrives late.
    let late = every_kind_of_event("toolu-old-late");
    let sent = late.len();
    cli.deliver(&old, late).await;
    drop(old);
    old_worker.await.expect("old worker");
    let (taken, effects) = cli.drain();

    assert_eq!(
        taken, sent,
        "{case}: every late event was delivered and judged"
    );
    assert!(effects.is_empty(), "{case}: late effects {effects:?}");
    assert_eq!(
        workspace(&cli.app),
        replaced,
        "{case}: a late event changed the new workspace"
    );
    // Spelled out, for a clearer failure than the comparison above.
    let shown = serde_json::to_string(&*cli.app.messages).expect("messages serialize");
    assert!(!shown.contains("late"), "{case}: late transcript {shown}");
    assert!(
        !cli.app.streaming_text.contains("late"),
        "{case}: late live text"
    );
    assert!(
        cli.app
            .tool_use_blocks
            .iter()
            .chain(cli.app.interrupted_tools.iter())
            .all(|block| block.id != "toolu-old-late"),
        "{case}: late tool row"
    );
    assert_ne!(
        cli.app.status_message.as_deref(),
        Some("late status"),
        "{case}"
    );
    assert_eq!(cli.app.context_used_tokens, 0, "{case}: late usage counted");

    // The new generation's own worker does reach the App.
    let (fresh, fresh_worker) = cli.submit("Fourth question.");
    cli.deliver(&fresh, vec![delta("fresh answer")]).await;
    drop(fresh);
    fresh_worker.await.expect("fresh worker");
    cli.drain();
    assert_eq!(cli.app.streaming_text.as_str(), "fresh answer", "{case}");
}

/// An open rewind holds every turn the loop would start on its own: remote
/// prompts and rate-limit retries (both by `rewind_flow.visible` at their
/// start sites) and idle compaction (here). A rewind confirmed while a turn
/// still runs is adopted only after that turn's join, so the join cannot
/// replace the adopted conversation with the worker's.
#[test]
fn a_rewind_holds_new_turns_and_waits_for_the_running_one() {
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        let case = format!("{renderer:?}");
        let mut cli = Interactive::resumed(renderer, history());
        cli.app.auto_compact_enabled = true;
        cli.app.context_window_size = 100;
        cli.app.context_used_tokens = 99;
        cli.app.auto_compact_threshold = 90;
        assert_eq!(select_idle_compaction(&cli.app, false), Some(99), "{case}");

        assert!(cli.app.intercept_slash_command_with_args("rewind", ""));
        assert_eq!(
            select_idle_compaction(&cli.app, false),
            None,
            "{case}: compaction started under the rewind"
        );
        press(&mut cli.app, crossterm::event::KeyCode::Up);
        press(&mut cli.app, crossterm::event::KeyCode::Enter);
        press(&mut cli.app, crossterm::event::KeyCode::Char('y'));
        assert!(!cli.app.rewind_flow.visible, "{case}");

        let generation = cli.scope.current();
        let before = texts(&cli.messages);
        adopt_in_app_rewind(
            &mut cli.scope,
            &cli.app,
            true,
            &mut cli.messages,
            &mut cli.session,
        );
        assert!(cli.scope.admits(generation), "{case}: adopted under a turn");
        assert_eq!(texts(&cli.messages), before, "{case}");
        assert_eq!(texts(&cli.session.messages), before, "{case}");

        // After the join, the next key adopts it.
        adopt_in_app_rewind(
            &mut cli.scope,
            &cli.app,
            false,
            &mut cli.messages,
            &mut cli.session,
        );
        assert!(!cli.scope.admits(generation), "{case}");
        let kept = ["First question.", "First answer."];
        assert_eq!(texts(&cli.messages), kept, "{case}");
        assert_eq!(texts(&cli.session.messages), kept, "{case}");
    }
}
