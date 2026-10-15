use claurst_commands::{CommandContext, CommandResult, SessionCommand, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker};

#[tokio::test]
async fn session_remote_url_handles_utf8_boundaries() {
    let cases = [
        format!("{}étail", "a".repeat(59)),
        format!("{}界tail", "a".repeat(59)),
        format!("{}\u{10000}tail", "a".repeat(58)),
        "a".repeat(61),
        format!("{}界", "a".repeat(57)),
        "https://example.test/界".to_string(),
    ];
    for (index, url) in cases.into_iter().enumerate() {
        if index < 3 {
            assert!(!url.is_char_boundary(60));
        }
        let mut prefix = String::new();
        for character in url.chars() {
            if prefix.len() + character.len_utf8() > 60 {
                break;
            }
            prefix.push(character);
        }
        if url.len() > 60 {
            prefix.push('…');
        }
        let border = "─".repeat(url.len().min(60) + 4);
        let expected = format!(
            "Remote session active\n┌{border}┐\n│  {prefix}  │\n└{border}┘\n\nOpen the URL above on any device to connect remotely.\nSession ID: fixture-session"
        );
        let original_url = url.clone();
        let mut context = CommandContext {
            config: Config::default(),
            cost_tracker: CostTracker::new(),
            messages: vec![],
            working_dir: std::path::PathBuf::from("unused-fixture-path"),
            session_id: "fixture-session".into(),
            session_title: None,
            remote_session_url: Some(url),
            mcp_manager: None,
            mcp_auth_runner: None,
        };
        let result = SessionCommand.execute("  ", &mut context).await;
        let CommandResult::Message(message) = result else {
            panic!("unexpected result: {result:?}");
        };
        assert_eq!(message, expected);
        assert_eq!(
            context.remote_session_url.as_deref(),
            Some(original_url.as_str())
        );
    }
}
