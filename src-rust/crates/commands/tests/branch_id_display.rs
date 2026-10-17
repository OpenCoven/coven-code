use claurst_commands::{
    named_commands::{BranchCommand, NamedCommand},
    CommandContext, CommandResult,
};
use claurst_core::{config::Config, cost::CostTracker, history, types::Message};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn branch_parent_id_preserves_utf8() {
    run_cases("parent");
}

#[test]
fn branch_child_id_preserves_utf8() {
    run_cases("child");
}

fn run_cases(target: &str) {
    let mut failures = Vec::new();
    for case in ["two", "three", "four", "ascii", "exact", "short"] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let empty_path = temp.path().join("empty-path");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&empty_path).unwrap();
        let log = temp.path().join("stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "branch_worker", "--ignored", "--nocapture"])
            .env_clear()
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("COVEN_CODE_HOME", &home)
            .env("COVEN_HOME", temp.path().join("coven"))
            .env("COVEN_CAVE_HOME", temp.path().join("cave"))
            .env("PATH", &empty_path)
            .env("BRANCH_TARGET", target)
            .env("BRANCH_CASE", case)
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
                    panic!("{target}/{case}: {result:?}");
                }
            }
        };
        if status.success() {
            assert_eq!(std::fs::read(home.join("done")).unwrap(), b"checked");
        } else {
            failures.push(format!(
                "{target}/{case}: {}",
                std::fs::read_to_string(log).unwrap()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn prefix(id: &str) -> String {
    let mut result = String::new();
    for character in id.chars() {
        if result.len() + character.len_utf8() > 8 {
            break;
        }
        result.push(character);
    }
    result
}

fn saved_files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

#[test]
#[ignore = "isolated process configuration"]
fn branch_worker() {
    let home = PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(claurst_core::Settings::config_dir(), home);
    let target = std::env::var("BRANCH_TARGET").unwrap();
    let case = std::env::var("BRANCH_CASE").unwrap();
    let id = match case.as_str() {
        "two" => format!("{}étail", "a".repeat(7)),
        "three" => format!("{}界tail", "a".repeat(7)),
        "four" => format!("{}\u{10000}tail", "a".repeat(6)),
        "ascii" => "a".repeat(9),
        "exact" => format!("{}界", "a".repeat(5)),
        "short" => "é界".into(),
        _ => panic!("unknown case"),
    };
    if matches!(case.as_str(), "two" | "three" | "four") {
        assert!(!id.is_char_boundary(8));
    }
    // These named-command bridges require a multi-thread runtime. Runtime
    // flavor compatibility is independent of display truncation.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut parent = history::ConversationSession::new("fixture-model".into());
        parent.id = if target == "parent" { id.clone() } else { "parent-ascii".into() };
        parent.title = Some("parent-title".into());
        parent.messages = vec![Message::user("first"), Message::user("second")];
        history::save_session(&parent).await.unwrap();
        assert_eq!(history::load_session(&parent.id).await.unwrap().id, parent.id);

        let child = if target == "parent" {
            let child = history::branch_session(&parent.id, 1, Some("child-title"))
                .await.unwrap();
            assert_eq!(child.messages.len(), 1);
            child
        } else {
            assert_eq!(target, "child");
            let mut child = history::ConversationSession::new("fixture-model".into());
            child.id = id.clone();
            child.title = Some("child-title".into());
            child.branch_from = Some(parent.id.clone());
            child.branch_at_message = Some(1);
            child.messages = vec![Message::user("first")];
            history::save_session(&child).await.unwrap();
            child
        };
        assert_eq!(child.branch_from.as_deref(), Some(parent.id.as_str()));
        assert_eq!(history::load_session(&child.id).await.unwrap().id, child.id);
        let mut unrelated = history::ConversationSession::new("fixture-model".into());
        unrelated.id = "unrelated-child".into();
        unrelated.branch_from = Some("another-parent".into());
        unrelated.title = Some("must-not-appear".into());
        history::save_session(&unrelated).await.unwrap();
        let before = saved_files(&home.join("sessions"));
        assert_eq!(before.len(), 3);
        let context = CommandContext {
            config: Config::default(), cost_tracker: CostTracker::new(),
            messages: vec![Message::user("current context")], working_dir: home.clone(),
            session_id: parent.id.clone(), session_title: Some("unchanged-title".into()),
            remote_session_url: None, mcp_manager: None, mcp_auth_runner: None,
        };
        let messages_before = serde_json::to_vec(&context.messages).unwrap();
        let result = BranchCommand.execute_named(&["list"], &context);
        let CommandResult::Message(message) = result else { panic!("{result:?}"); };
        let date = child.updated_at.format("%Y-%m-%d %H:%M");
        assert_eq!(message, format!(
            "Branches of session {}:\n\n  {} | {date} | 1 messages | child-title\n\nUse: coven-code branch switch <id>",
            prefix(&parent.id), prefix(&child.id),
        ));
        assert_eq!(saved_files(&home.join("sessions")), before);
        assert_eq!(context.session_id, parent.id);
        assert_eq!(context.session_title.as_deref(), Some("unchanged-title"));
        assert_eq!(serde_json::to_vec(&context.messages).unwrap(), messages_before);
    });
    std::fs::write(home.join("done"), b"checked").unwrap();
}
