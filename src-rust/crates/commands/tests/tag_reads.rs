use claurst_commands::{
    named_commands::{NamedCommand, TagCommand},
    CommandContext, CommandResult,
};
use claurst_core::{config::Config, cost::CostTracker, history, types::Message};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn tag_list_reports_read_errors_without_changes() {
    run_cases(&["syntax", "type", "utf8", "directory", "pre-session"]);
}

#[test]
fn tag_list_preserves_missing_and_valid_results() {
    run_cases(&["missing", "empty", "tags"]);
}

fn run_cases(cases: &[&str]) {
    let mut failures = Vec::new();
    for case in cases {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let log = temp.path().join("stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tag_read_worker", "--ignored", "--nocapture"])
            .env_clear()
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("COVEN_CODE_HOME", &home)
            .env("COVEN_HOME", temp.path().join("coven"))
            .env("COVEN_CAVE_HOME", temp.path().join("cave"))
            .env("TAG_CASE", case)
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
                    std::thread::sleep(Duration::from_millis(10));
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{case}: child wait failed: {result:?}");
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
#[ignore = "owned process configuration and history files"]
fn tag_read_worker() {
    let home = PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(claurst_core::Settings::config_dir(), home);
    let case = std::env::var("TAG_CASE").unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let id = if case == "pre-session" {
            "pre-session"
        } else {
            "fixture-session"
        };
        let path = home.join("sessions").join(format!("{id}.json"));
        let marker = "782346981235670123";
        let mut session = history::ConversationSession::new("fixture-model".into());
        session.id = id.into();
        session.tags = if case == "tags" {
            vec!["first".into(), "é space".into()]
        } else {
            vec![]
        };
        match case.as_str() {
            "missing" => {}
            "directory" => {
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(path.join("sentinel"), b"untouched").unwrap();
            }
            _ => {
                history::save_session(&session).await.unwrap();
                match case.as_str() {
                    "syntax" | "pre-session" => {
                        std::fs::write(&path, format!("{{\"private\":\"{marker}\",")).unwrap()
                    }
                    "type" => {
                        let mut value = serde_json::to_value(&session).unwrap();
                        value["model"] = serde_json::json!(782346981235670123u64);
                        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
                    }
                    "utf8" => std::fs::write(&path, [0xff, 0xfe]).unwrap(),
                    "empty" | "tags" => {}
                    _ => panic!("unknown case"),
                }
            }
        }
        let before = if path.is_file() {
            Some(std::fs::read(&path).unwrap())
        } else {
            None
        };
        let loaded = history::load_session(id).await;
        let expected_error = match case.as_str() {
            "syntax" | "type" | "pre-session" => {
                let error = loaded.unwrap_err();
                let json = error.downcast_ref::<serde_json::Error>().unwrap();
                assert_eq!(
                    json.classify(),
                    if case == "type" {
                        serde_json::error::Category::Data
                    } else {
                        serde_json::error::Category::Eof
                    }
                );
                if case == "type" {
                    assert!(json.to_string().contains(marker));
                }
                Some(format!(
                    "invalid session JSON ({:?}) at line {} column {}",
                    json.classify(),
                    json.line(),
                    json.column()
                ))
            }
            "utf8" | "directory" | "missing" => {
                let error = loaded.unwrap_err();
                let io = error.downcast_ref::<std::io::Error>().unwrap();
                if case == "missing" {
                    assert_eq!(io.kind(), std::io::ErrorKind::NotFound);
                } else if case == "utf8" {
                    assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);
                } else {
                    assert_ne!(io.kind(), std::io::ErrorKind::NotFound);
                }
                Some(io.to_string())
            }
            _ => {
                assert_eq!(loaded.unwrap().tags, session.tags);
                None
            }
        };
        let context = CommandContext {
            config: Config::default(),
            cost_tracker: CostTracker::new(),
            messages: vec![Message::user("unchanged context")],
            working_dir: home.clone(),
            session_id: id.into(),
            session_title: Some("unchanged title".into()),
            remote_session_url: None,
            mcp_manager: None,
            mcp_auth_runner: None,
        };
        let messages = serde_json::to_vec(&context.messages).unwrap();
        let mut failures = Vec::new();
        for args in [vec![], vec!["list"]] {
            let result = TagCommand.execute_named(&args, &context);
            if let Some(bytes) = &before {
                assert_eq!(&std::fs::read(&path).unwrap(), bytes);
            } else if case == "directory" {
                assert!(path.is_dir());
                assert_eq!(std::fs::read(path.join("sentinel")).unwrap(), b"untouched");
                assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
            } else {
                assert!(!home.join("sessions").exists());
            }
            assert_eq!(context.session_id, id);
            assert_eq!(context.session_title.as_deref(), Some("unchanged title"));
            assert_eq!(serde_json::to_vec(&context.messages).unwrap(), messages);
            match case.as_str() {
                "missing" | "empty" | "tags" => {
                    let expected = match case.as_str() {
                        "missing" => "No tags set for this session (session not yet saved).",
                        "empty" => "No tags set for this session.",
                        _ => "Tags for this session:\n  #first\n  #é space",
                    };
                    assert!(
                        matches!(result, CommandResult::Message(ref m) if m == expected),
                        "{result:?}"
                    );
                }
                _ => match result {
                    CommandResult::Error(message) => {
                        assert!(message.contains("Could not list tags"), "{message}");
                        assert!(message.contains(id), "{message}");
                        assert!(
                            message.contains(expected_error.as_ref().unwrap()),
                            "{message}"
                        );
                        assert!(!message.contains(marker), "private value leaked");
                    }
                    result => failures.push(format!("{args:?}: {result:?}")),
                },
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    });
    std::fs::write(home.join("done"), b"checked").unwrap();
}
