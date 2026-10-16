use claurst_commands::{CommandContext, CommandResult, SearchCommand, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker, SqliteSessionStore};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn search_dates_preserve_utf8_and_storage() {
    let mut failures = Vec::new();
    for case in ["two", "three", "four", "ascii", "exact", "short", "normal"] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let log = temp.path().join("stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "search_date_worker", "--ignored", "--nocapture"])
            .env_clear()
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("COVEN_CODE_HOME", &home)
            .env("COVEN_HOME", home.join("coven"))
            .env("COVEN_CAVE_HOME", home.join("cave"))
            .env("PATH", &home)
            .env("DATE_CASE", case)
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
                    panic!("{case}: {result:?}");
                }
            }
        };
        if status.success() {
            assert_eq!(std::fs::read(home.join("done")).unwrap(), b"checked");
        } else {
            failures.push(format!("{case}: {}", std::fs::read_to_string(log).unwrap()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "isolated process configuration"]
fn search_date_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(claurst_core::Settings::config_dir(), home);
    let case = std::env::var("DATE_CASE").unwrap();
    let date = match case.as_str() {
        "two" => format!("{}étail", "a".repeat(9)),
        "three" => format!("{}界tail", "a".repeat(9)),
        "four" => format!("{}\u{10000}tail", "a".repeat(8)),
        "ascii" => "a".repeat(11),
        "exact" => format!("{}界", "a".repeat(7)),
        "short" => "é界".into(),
        "normal" => "2026-10-15T10:20:30+00:00".into(),
        _ => panic!("unknown case"),
    };
    if matches!(case.as_str(), "two" | "three" | "four") {
        assert!(!date.is_char_boundary(10));
    }
    let mut prefix = String::new();
    for character in date.chars() {
        if prefix.len() + character.len_utf8() > 10 {
            break;
        }
        prefix.push(character);
    }
    let path = home.join("sessions.db");
    let store = SqliteSessionStore::open(&path).unwrap();
    store
        .save_session("fixture", Some("fixture-title"), "fixture-model")
        .unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE sessions SET updated_at=?1 WHERE id='fixture'",
            [&date],
        )
        .unwrap();
    drop(connection);
    drop(store);
    let bytes = std::fs::read(&path).unwrap();
    let mut context = CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: vec![],
        working_dir: home.clone(),
        session_id: "context-id".into(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(SearchCommand.execute("fixture-title", &mut context));
    let CommandResult::Message(message) = result else {
        panic!("{result:?}");
    };
    assert_eq!(message, format!("Search results for \"fixture-title\": 1 session(s)\n\n  [fixture] fixture-title — fixture-model (0 messages, updated {prefix})\n\nTip: use /resume <session-id> to continue a session."));
    assert_eq!(context.session_id, "context-id");
    assert!(context.messages.is_empty());
    assert_eq!(context.working_dir, home);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    std::fs::write(home.join("done"), b"checked").unwrap();
}
