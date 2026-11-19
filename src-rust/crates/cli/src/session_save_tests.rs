use super::*;
use claurst_tui::notifications::{NotificationKind, NotificationQueue};

/// Runs one ignored worker test of this module in a child process whose
/// config home is a fresh temporary directory, and checks that it finished.
fn run_isolated_worker(worker: &str) {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("stderr");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("session_save_tests::{worker}"),
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("COVEN_CODE_HOME", temp.path())
        .env("COVEN_HOME", temp.path().join("coven"))
        .env("COVEN_CAVE_HOME", temp.path().join("cave"))
        .env("HOME", temp.path())
        .env("USERPROFILE", temp.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("bounded session save worker: {result:?}");
            }
        }
    };
    assert!(
        status.success(),
        "{}",
        std::fs::read_to_string(log).unwrap()
    );
    assert_eq!(std::fs::read(temp.path().join("done")).unwrap(), b"checked");
}

#[test]
fn session_save_failures_and_recovery_are_visible() {
    run_isolated_worker("session_save_worker");
}

#[test]
fn session_index_keeps_every_user_message_once() {
    run_isolated_worker("session_index_worker");
}

#[test]
#[ignore = "isolated session save environment"]
fn session_save_worker() {
    let home = PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(Settings::config_dir(), home);
    let diagnostic = home.join("diagnostic");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(std::sync::Mutex::new(
            std::fs::File::create(&diagnostic).unwrap(),
        ))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for (index, operation) in [
            "rename",
            "remote connection",
            "remote rename",
            "turn completion",
            "transcript update",
        ]
        .iter()
        .enumerate()
        {
            let mut session =
                claurst_core::history::ConversationSession::new("fixture-model".into());
            session.id = format!("fixture-{index}");
            session.title = Some("private-title-marker".into());
            session
                .messages
                .push(claurst_core::types::Message::user("private-message-marker"));
            let before = serde_json::to_vec_pretty(&session).unwrap();
            let path = home.join("sessions").join(format!("{}.json", session.id));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("sentinel"), b"untouched").unwrap();
            let cause = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap_err()
                .to_string();
            let mut notifications = NotificationQueue::new();
            notifications.push(NotificationKind::Warning, "unrelated warning".into(), None);
            for _ in 0..2 {
                if *operation == "transcript update" {
                    assert!(!persist_interactive_session(&session, None, &mut notifications).await);
                } else {
                    assert!(
                        !save_session_with_warning(&session, &mut notifications, operation).await
                    );
                }
            }
            assert_eq!(notifications.notifications.len(), 2);
            let warning = notifications.current().unwrap();
            assert_eq!(warning.kind, NotificationKind::Warning);
            assert_eq!(warning.message, SESSION_SAVE_WARNING);
            assert!(warning.expires_at.is_none());
            notifications.tick();
            assert!(!notifications.current_is_error());
            assert_eq!(std::fs::read(path.join("sentinel")).unwrap(), b"untouched");
            assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
            assert_eq!(serde_json::to_vec_pretty(&session).unwrap(), before);
            let logs = std::fs::read_to_string(&diagnostic).unwrap();
            assert!(logs.contains("Failed to persist session"));
            assert!(logs.contains(&session.id));
            assert!(logs.contains(operation));
            assert!(logs.contains(&cause));
            assert!(!logs.contains("private-title-marker"));
            assert!(!logs.contains("private-message-marker"));
            std::fs::remove_file(path.join("sentinel")).unwrap();
            std::fs::remove_dir(&path).unwrap();
            if *operation == "transcript update" {
                assert!(persist_interactive_session(&session, None, &mut notifications).await);
            } else {
                assert!(save_session_with_warning(&session, &mut notifications, operation).await);
            }
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert_eq!(notifications.notifications.len(), 1);
            assert_eq!(
                notifications.current().unwrap().message,
                "unrelated warning"
            );
            assert_eq!(std::fs::read_to_string(&diagnostic).unwrap(), logs);
        }
    });
    std::fs::write(home.join("done"), b"checked").unwrap();
}

#[test]
#[ignore = "isolated session save environment"]
fn session_index_worker() {
    use claurst_core::types::Message;

    let home = PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(Settings::config_dir(), home);
    let index_path = home.join("sessions.db");
    // An index written by an earlier build: its first user message ever was
    // stored under the id "unknown".
    {
        let index = claurst_core::SqliteSessionStore::open(&index_path).unwrap();
        index.save_session("legacy", None, "fixture-model").unwrap();
        index
            .save_message("legacy", "unknown", "user", "legacy-first-prompt", None)
            .unwrap();
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut notifications = NotificationQueue::new();
        let mut legacy = claurst_core::history::ConversationSession::new("fixture-model".into());
        legacy.id = "legacy".into();
        legacy.messages = vec![
            Message::user("legacy-first-prompt"),
            Message::user("legacy-second-prompt"),
        ];
        let mut answer = Message::assistant("first-answer");
        answer.uuid = Some("answer-1".into());
        let mut first = claurst_core::history::ConversationSession::new("fixture-model".into());
        first.id = "first".into();
        first.messages = vec![
            Message::user("first-prompt-one"),
            answer,
            Message::user("first-prompt-two"),
            // The same text typed twice is two messages.
            Message::user("first-prompt-one"),
        ];
        let mut second = claurst_core::history::ConversationSession::new("fixture-model".into());
        second.id = "second".into();
        second.messages = vec![Message::user("other-session-prompt")];
        // Every turn saves the whole session again.
        for session in [&legacy, &first, &first, &second, &second, &legacy] {
            assert!(persist_interactive_session(session, None, &mut notifications).await);
        }
    });

    let index = claurst_core::SqliteSessionStore::open(&index_path).unwrap();
    for (text, session_id) in [
        ("legacy-first-prompt", "legacy"),
        ("legacy-second-prompt", "legacy"),
        ("first-prompt-one", "first"),
        ("first-prompt-two", "first"),
        ("first-answer", "first"),
        ("other-session-prompt", "second"),
    ] {
        let found: Vec<_> = index
            .search_sessions(text)
            .unwrap()
            .into_iter()
            .map(|summary| summary.id)
            .collect();
        assert_eq!(found, [session_id], "search for {text}");
    }
    // `message_count` grows only when a row is inserted, so it counts rows.
    let mut counts: Vec<_> = index
        .list_sessions()
        .unwrap()
        .into_iter()
        .map(|summary| (summary.id, summary.message_count))
        .collect();
    counts.sort();
    assert_eq!(
        counts,
        [
            ("first".to_owned(), 4),
            ("legacy".to_owned(), 2),
            ("second".to_owned(), 1)
        ]
    );
    std::fs::write(home.join("done"), b"checked").unwrap();
}
