use claurst_commands::{
    CommandContext, CommandResult, SearchCommand, SessionCommand, SlashCommand, StatusCommand,
};
use claurst_core::{config::Config, cost::CostTracker, history, SqliteSessionStore};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn session_id_displays_preserve_utf8_and_storage() {
    let mut failures = Vec::new();
    for operation in ["status", "list", "recent", "search"] {
        for case in ["two", "three", "four", "ascii", "exact", "short"] {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            let empty_path = temp.path().join("empty-path");
            std::fs::create_dir(&home).unwrap();
            std::fs::create_dir(&empty_path).unwrap();
            let log = temp.path().join("stderr");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "session_id_worker", "--ignored", "--nocapture"])
                .env_clear()
                .env("HOME", &home)
                .env("USERPROFILE", &home)
                .env("COVEN_CODE_HOME", &home)
                .env("COVEN_HOME", temp.path().join("coven"))
                .env("COVEN_CAVE_HOME", temp.path().join("cave"))
                .env("PATH", &empty_path)
                .env("ID_OPERATION", operation)
                .env("ID_CASE", case)
                .current_dir(&home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(std::fs::File::create(&log).unwrap())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    result => {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("{operation}/{case}: {result:?}");
                    }
                }
            };
            if status.success() {
                assert_eq!(std::fs::read(home.join("done")).unwrap(), b"checked");
            } else {
                failures.push(format!(
                    "{operation}/{case}: {}",
                    std::fs::read_to_string(log).unwrap()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "isolated process configuration"]
fn session_id_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(claurst_core::Settings::config_dir(), home);
    let operation = std::env::var("ID_OPERATION").unwrap();
    let case = std::env::var("ID_CASE").unwrap();
    let budget = if matches!(operation.as_str(), "list" | "recent") {
        8
    } else {
        12
    };
    let id = match case.as_str() {
        "two" => format!("{}étail", "a".repeat(budget - 1)),
        "three" => format!("{}界tail", "a".repeat(budget - 1)),
        "four" => format!("{}\u{10000}tail", "a".repeat(budget - 2)),
        "ascii" => "a".repeat(budget + 1),
        "exact" => format!("{}界", "a".repeat(budget - 3)),
        "short" => "é界".into(),
        _ => panic!("unknown case"),
    };
    if matches!(case.as_str(), "two" | "three" | "four") {
        assert!(!id.is_char_boundary(budget));
    }
    let mut prefix = String::new();
    for character in id.chars() {
        if prefix.len() + character.len_utf8() > budget {
            break;
        }
        prefix.push(character);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut session = history::ConversationSession::new("fixture-model".into());
        session.id = id.clone();
        session.title = Some("fixture-title".into());
        history::save_session(&session).await.unwrap();
        let history_path = home.join("sessions").join(format!("{id}.json"));
        let original_history = std::fs::read(&history_path).unwrap();
        let store = SqliteSessionStore::open(&home.join("sessions.db")).unwrap();
        store
            .save_session(&id, Some("fixture-title"), "fixture-model")
            .unwrap();
        let before = store.list_sessions().unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].id, id);
        assert_eq!(history::load_session(&id).await.unwrap().id, id);
        let mut context = CommandContext {
            config: Config::default(),
            cost_tracker: CostTracker::new(),
            messages: vec![],
            working_dir: home.clone(),
            session_id: id.clone(),
            session_title: None,
            remote_session_url: None,
            mcp_manager: None,
            mcp_auth_runner: None,
        };
        let result = match operation.as_str() {
            "status" => StatusCommand.execute("", &mut context).await,
            "list" => SessionCommand.execute("list", &mut context).await,
            "recent" => SessionCommand.execute("", &mut context).await,
            "search" => SearchCommand.execute("fixture-title", &mut context).await,
            _ => panic!("unknown operation"),
        };
        let CommandResult::Message(message) = result else {
            panic!("{result:?}");
        };
        match operation.as_str() {
            "status" => {
                assert!(
                    message
                        .lines()
                        .any(|line| line == format!("Session ID:     {prefix}")),
                    "{message}"
                );
                assert!(message.contains("Auth:           Not authenticated\n"));
                assert!(message.contains("Git branch:     n/a\n"));
            }
            "list" | "recent" => {
                let date = session.updated_at.format("%Y-%m-%d %H:%M");
                let marker = if operation == "recent" {
                    " ◀ current"
                } else {
                    ""
                };
                let row = format!("  {prefix} | {date} | 0 messages | fixture-title{marker}");
                assert!(message.lines().any(|line| line == row), "{message}");
                if operation == "recent" {
                    assert!(message
                        .lines()
                        .any(|line| line == format!("ID:       {id}")));
                }
            }
            "search" => {
                let date: String = before[0].updated_at.chars().take(10).collect();
                let row = format!(
                    "  [{prefix}] fixture-title — fixture-model (0 messages, updated {date})"
                );
                assert!(message.lines().any(|line| line == row), "{message}");
            }
            _ => unreachable!(),
        }
        assert_eq!(context.session_id, id);
        assert_eq!(std::fs::read(history_path).unwrap(), original_history);
        assert_eq!(history::load_session(&id).await.unwrap().id, id);
        let after = store.list_sessions().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, id);
        assert_eq!(after[0].title, before[0].title);
        assert_eq!(after[0].model, before[0].model);
        assert_eq!(after[0].created_at, before[0].created_at);
        assert_eq!(after[0].updated_at, before[0].updated_at);
        assert_eq!(after[0].message_count, before[0].message_count);
    });
    std::fs::write(home.join("done"), b"checked").unwrap();
}
