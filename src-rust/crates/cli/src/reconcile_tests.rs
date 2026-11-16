//! #408: "Late events for a previous session/view generation cannot update a
//! new workspace."
//!
//! The interactive loop tags each worker's events with the generation it was
//! spawned in (`forward_scoped_events`), and admits only the current
//! generation to the App (`SessionScope::apply` in its drain). Clearing or
//! rewinding the conversation starts a new generation through
//! `replace_conversation`. These tests put those real pieces together around
//! a worker that is still delivering when the conversation is replaced, and
//! check that nothing it sends afterwards reaches the new workspace in either
//! renderer: not its transcript, tool rows, status, streaming state or
//! notifications, not an effect for the loop, and not the conversation the
//! loop sends or saves.
//!
//! The loop runs `/clear` and `/rewind <n>` only once no worker is running
//! (input is queued while one is), so there the filter is a second guard; the
//! guarantee is the filter's, and it is tested here, where it is applied.

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
    fn new(renderer: UiRenderer) -> Self {
        let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
        app.select_renderer(renderer);
        app.bell_on_complete = true;
        app.completion_toast_min_secs = 0;
        let (events, queue) =
            tokio::sync::mpsc::channel(claurst_tui::application::QUERY_EVENT_QUEUE_CAPACITY);
        Self {
            app,
            messages: Vec::new(),
            session: claurst_core::history::ConversationSession::new("model".into()),
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

/// How the conversation is replaced.
#[derive(Debug, Clone, Copy)]
enum Replacement {
    /// `/clear`: the App's own clear, then the CLI's.
    Clear,
    /// `/rewind <n>`: the CLI sets the conversation to its first message.
    Rewind,
}

impl Replacement {
    fn perform(self, loop_state: &mut Interactive) {
        let Interactive {
            app,
            messages,
            session,
            scope,
            ..
        } = loop_state;
        match self {
            Replacement::Clear => {
                // Step 1 of the slash-command path, then the
                // `CommandResult::ClearConversation` arm.
                assert!(app.intercept_slash_command_with_args("clear", ""));
                replace_conversation(scope, Vec::new(), messages, app, session);
                app.status_message = Some("Conversation cleared.".to_string());
            }
            Replacement::Rewind => {
                let kept = messages[..1].to_vec();
                replace_conversation(scope, kept, messages, app, session);
            }
        }
    }
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
    let mut cli = Interactive::new(renderer);
    let (first, first_worker) = cli.submit("First question.");
    for event in [
        delta("first answer"),
        QueryEvent::TurnComplete {
            turn: 1,
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ] {
        first.send(event).expect("worker is running");
    }
    drop(first);
    first_worker.await.expect("first worker");
    cli.drain();
    // As the loop does when it joins the worker.
    cli.app.is_streaming = false;

    // The second turn's worker is still delivering when the
    // conversation is replaced.
    let (old, old_worker) = cli.submit("Second question.");
    let old_generation: SessionGeneration = cli.scope.current();
    for event in [
        delta("partial "),
        QueryEvent::ToolStart {
            tool_name: "Bash".to_string(),
            tool_id: "toolu-old-running".to_string(),
            input_json: r#"{"command":"cargo test"}"#.to_string(),
        },
    ] {
        old.send(event).expect("worker is running");
    }
    tokio::task::yield_now().await;
    let (taken, _) = cli.drain();
    assert_eq!(taken, 2, "{case}: the turn reached the App before");
    assert!(!cli.app.tool_use_blocks.is_empty(), "{case}");

    replacement.perform(&mut cli);
    assert!(
        !cli.scope.admits(old_generation),
        "{case}: the replacement started a new generation"
    );
    let replaced = workspace(&cli.app);
    let kept = cli.messages.clone();
    let saved = cli.session.messages.clone();

    // Everything the old worker still sends arrives late.
    let late = every_kind_of_event("toolu-old-late");
    let sent = late.len();
    for event in late {
        old.send(event).expect("worker is running");
    }
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
    let texts = |messages: &[Message]| -> Vec<String> {
        messages.iter().map(Message::get_all_text).collect()
    };
    assert_eq!(
        texts(&cli.messages),
        texts(&kept),
        "{case}: sent conversation"
    );
    assert_eq!(
        texts(&cli.session.messages),
        texts(&saved),
        "{case}: saved conversation"
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
    match replacement {
        Replacement::Clear => {
            assert!(cli.app.messages.is_empty(), "{case}");
            assert!(cli.app.tool_use_blocks.is_empty(), "{case}");
            assert_eq!(
                cli.app.status_message.as_deref(),
                Some("Conversation cleared."),
                "{case}"
            );
        }
        Replacement::Rewind => {
            assert_eq!(texts(&cli.app.messages), ["First question."], "{case}");
        }
    }

    // The new generation's own worker does reach the App.
    let (fresh, fresh_worker) = cli.submit("Third question.");
    fresh
        .send(delta("fresh answer"))
        .expect("worker is running");
    drop(fresh);
    fresh_worker.await.expect("fresh worker");
    cli.drain();
    assert_eq!(cli.app.streaming_text.as_str(), "fresh answer", "{case}");
}
