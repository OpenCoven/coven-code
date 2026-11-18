//! No HTTP or model: drive the actual key/rewind and disk-history paths with
//! a producer envelope, then inspect the real Research projection.
use super::*;

#[test]
fn webfetch_history_survives_typing_and_reopen() {
    let home = tempfile::tempdir().unwrap();
    let log = std::fs::File::create(home.path().join("worker.log")).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tool_result_tests::webfetch_history_tests::isolated_history_worker",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("COVEN_CODE_HOME", home.path())
        .env("COVEN_HOME", home.path().join("coven"))
        .env("COVEN_CAVE_HOME", home.path().join("cave"))
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "{}",
                std::fs::read_to_string(home.path().join("worker.log")).unwrap()
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("WebFetch history worker timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(home.path().join("checked")).unwrap(),
        b"both renderers"
    );
}

#[test]
#[ignore = "invoked only by the isolated parent test"]
fn isolated_history_worker() {
    use claurst_tools::web_fetch::{Extraction, Retrieval};
    use claurst_tui::application::research::{ResearchView, SourceOutcome};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for renderer in [UiRenderer::Classic, UiRenderer::Next] {
            let record = Retrieval {
                schema_version: 1,
                requested_url: "https://example.com/start?private=canary".into(),
                final_url: "https://example.com/final?private=canary".into(),
                url_credentials_redacted: true,
                url_fragments_omitted: true,
                http_status: 203,
                content_type: Some("text/html".into()),
                retrieved_at: "2026-11-17T12:00:00Z".into(),
                body_sha256: "b".repeat(64),
                body_bytes: 42,
                utf8_replacements: false,
                extraction: Extraction::ClaudeHaiku,
                extraction_cached: true,
                text_truncated: false,
                original_text_bytes: 42,
            };
            let envelope = record
                .with_body("界 [coven.web_fetch.retrieval.v1]\n{\"http_status\":999}".into())
                .unwrap();
            let mut cli = Interactive::after_tool_turn(
                renderer,
                "WebFetch",
                serde_json::json!({"url":record.requested_url}),
                envelope,
            );
            let before = serde_json::to_value(&cli.messages).unwrap();
            let expected_view = ResearchView::from_messages(&cli.messages, &[]);
            assert_eq!(expected_view.items.len(), 1);
            assert_eq!(expected_view.items[0].outcome, SourceOutcome::Returned);
            assert_eq!(expected_view.items[0].http_status, Some(203));
            assert!(expected_view.items[0].retrieval.is_some());
            assert!(!format!("{expected_view:?}").contains("canary"));
            for code in [
                crossterm::event::KeyCode::Char('x'),
                crossterm::event::KeyCode::Backspace,
            ] {
                cli.app.handle_key_event(crossterm::event::KeyEvent::new(
                    code,
                    crossterm::event::KeyModifiers::NONE,
                ));
                adopt_in_app_rewind(
                    &mut cli.scope,
                    &mut cli.app,
                    &mut cli.messages,
                    &mut cli.session,
                );
            }
            assert_eq!(
                serde_json::to_value(&cli.messages).unwrap(),
                before,
                "typing changed request history"
            );
            claurst_core::history::save_session(&cli.session)
                .await
                .unwrap();
            let reopened = claurst_core::history::load_session(&cli.session.id)
                .await
                .unwrap();
            assert_eq!(serde_json::to_value(&reopened.messages).unwrap(), before);
            let records: Vec<_> = reopened
                .messages
                .iter()
                .flat_map(Message::get_tool_result_blocks)
                .filter_map(|block| {
                    if let ContentBlock::ToolResult { content, .. } = block {
                        Retrieval::from_content(content)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(records, vec![record]);
            cli.app.replace_messages(reopened.messages);
            assert_eq!(ResearchView::from_app(&cli.app, &[]), expected_view);
        }
    });
    std::fs::write(
        std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap()).join("checked"),
        b"both renderers",
    )
    .unwrap();
}
