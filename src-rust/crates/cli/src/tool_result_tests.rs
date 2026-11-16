//! After a turn that called a tool, the conversation the interactive loop
//! sends and saves keeps the message that carries the tool's result. The
//! App shows that result on the call's card instead of storing the message,
//! so its transcript is shorter than the loop's, and that is no rewind. A
//! rewind the user confirms in the overlay still cuts both at the same
//! message.

use super::*;
use claurst_core::types::{ContentBlock, Message, ToolResultContent};
use claurst_query::QueryEvent;
use claurst_tui::application::{ScopedQueryEvent, SessionScope, UiRenderer};

/// What the interactive loop holds for one session: the App, the
/// conversation it sends and saves, the session and the generation.
struct Interactive {
    app: claurst_tui::App,
    messages: Vec<Message>,
    session: claurst_core::history::ConversationSession,
    scope: SessionScope,
}

impl Interactive {
    /// A resumed session, set up as the loop sets it up, that has then run
    /// one turn calling a tool, through to the turn's join.
    fn after_a_tool_turn(renderer: UiRenderer) -> Self {
        let history = vec![
            Message::user("First question.".to_string()),
            Message::assistant("First answer.".to_string()),
            Message::user("Second question.".to_string()),
            Message::assistant("Second answer.".to_string()),
        ];
        let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
        app.select_renderer(renderer);
        let mut session = claurst_core::history::ConversationSession::new("model".into());
        session.messages = history.clone();
        app.replace_messages(history.clone());
        app.enter_session_scope(&session.id);
        let mut cli = Self {
            app,
            messages: history,
            session,
            scope: SessionScope::default(),
        };

        let prompt = Message::user("Run the tests.".to_string());
        cli.messages.push(prompt.clone());
        cli.app.push_message(prompt);
        cli.session.messages = cli.messages.clone();
        cli.app.is_streaming = true;

        let mut call = Message::assistant_blocks(vec![ContentBlock::ToolUse {
            id: "toolu-1".to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({ "command": "cargo test" }),
        }]);
        call.uuid = Some("msg-call".to_string());
        let result = Message::user_blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "toolu-1".to_string(),
            content: ToolResultContent::Text("ok".to_string()),
            is_error: None,
        }]);
        let mut answer = Message::assistant("The tests pass.".to_string());
        answer.uuid = Some("msg-answer".to_string());
        let events = [
            QueryEvent::ToolStart {
                tool_name: "Bash".to_string(),
                tool_id: "toolu-1".to_string(),
                input_json: r#"{"command":"cargo test"}"#.to_string(),
            },
            QueryEvent::ToolEnd {
                tool_name: "Bash".to_string(),
                tool_id: "toolu-1".to_string(),
                result: "ok".to_string(),
                is_error: false,
            },
            QueryEvent::DurableMessage {
                message: call.clone(),
            },
            QueryEvent::DurableMessage {
                message: result.clone(),
            },
            QueryEvent::DurableMessage {
                message: answer.clone(),
            },
            QueryEvent::TurnComplete {
                turn: 1,
                stop_reason: "end_turn".to_string(),
                usage: None,
            },
        ];
        // The loop's drain, in the turn's generation.
        for event in events {
            let scoped = ScopedQueryEvent {
                generation: cli.scope.current(),
                event,
            };
            cli.scope.apply(&mut cli.app, scoped);
        }
        assert!(!cli.app.is_streaming);

        // The join takes the worker's transcript, the result included.
        cli.messages.extend([call, result, answer]);
        cli.session.messages = cli.messages.clone();
        cli
    }
}

fn holds_the_tool_result(conversation: &[Message]) -> bool {
    conversation
        .iter()
        .any(|message| !message.get_tool_result_blocks().is_empty())
}

#[test]
fn a_turn_that_called_a_tool_is_not_taken_for_a_rewind() {
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        let case = format!("{renderer:?}");
        let mut cli = Interactive::after_a_tool_turn(renderer);
        assert!(cli.app.messages.len() < cli.messages.len(), "{case}");

        let generation = cli.scope.current();
        // The loop runs the adoption after every key.
        adopt_in_app_rewind(
            &mut cli.scope,
            &mut cli.app,
            &mut cli.messages,
            &mut cli.session,
        );

        assert!(cli.scope.admits(generation), "{case}: taken for a rewind");
        for (what, conversation) in [("sent", &cli.messages), ("saved", &cli.session.messages)] {
            assert_eq!(conversation.len(), 8, "{case}: {what}");
            assert!(
                holds_the_tool_result(conversation),
                "{case}: the {what} conversation lost the tool result"
            );
        }
    }
}

#[test]
fn a_confirmed_rewind_cuts_both_conversations_at_the_chosen_message() {
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        let case = format!("{renderer:?}");
        let mut cli = Interactive::after_a_tool_turn(renderer);

        // `/rewind`, as the loop runs it: the App's intercept, then the
        // `CommandResult::OpenRewindOverlay` arm.
        assert!(cli.app.intercept_slash_command_with_args("rewind", ""));
        cli.app.replace_messages(cli.messages.clone());
        cli.app.open_rewind_flow();

        // Rewind to the answer: the call and its result stay.
        let generation = cli.scope.current();
        for code in [
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyCode::Char('y'),
        ] {
            cli.app.handle_key_event(crossterm::event::KeyEvent::new(
                code,
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        assert!(!cli.app.rewind_flow.visible, "{case}");
        adopt_in_app_rewind(
            &mut cli.scope,
            &mut cli.app,
            &mut cli.messages,
            &mut cli.session,
        );

        assert!(!cli.scope.admits(generation), "{case}: a new generation");
        assert_eq!(cli.messages.len(), 7, "{case}");
        assert_eq!(
            serde_json::to_string(&cli.messages).expect("messages serialize"),
            serde_json::to_string(&*cli.app.messages).expect("messages serialize"),
            "{case}: the loop and the App keep the same messages"
        );
        assert_eq!(cli.session.messages.len(), 7, "{case}");
        assert!(holds_the_tool_result(&cli.messages), "{case}");

        // The next key finds no rewind left to adopt.
        let adopted = cli.scope.current();
        adopt_in_app_rewind(
            &mut cli.scope,
            &mut cli.app,
            &mut cli.messages,
            &mut cli.session,
        );
        assert!(cli.scope.admits(adopted), "{case}: adopted twice");
        assert_eq!(cli.messages.len(), 7, "{case}");
    }
}
