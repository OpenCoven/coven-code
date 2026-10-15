use claurst_commands::{CommandContext, CommandResult, MemoryCommand, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn memory_display_preserves_utf8() {
    run_cases(&["two", "three", "four"]);
}
#[test]
fn memory_display_size_controls() {
    run_cases(&["ascii", "exact", "short"]);
}

fn run_cases(cases: &[&str]) {
    let mut failures = Vec::new();
    for case in cases {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&project).unwrap();
        let log = temp.path().join("stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "memory_display_worker",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("COVEN_CODE_HOME", &home)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("MEMORY_FIXTURE_PROJECT", &project)
            .env("MEMORY_FIXTURE_CASE", case)
            .env("MEMORY_FIXTURE_DONE", temp.path().join("done"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{case}: child wait failed: {result:?}");
                }
            }
        };
        if outcome.success() {
            assert_eq!(std::fs::read(temp.path().join("done")).unwrap(), b"checked");
        } else {
            failures.push(format!("{case}: {}", std::fs::read_to_string(log).unwrap()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "owned homes and memory files"]
fn memory_display_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("MEMORY_FIXTURE_PROJECT").unwrap());
    let case = std::env::var("MEMORY_FIXTURE_CASE").unwrap();
    assert_eq!(claurst_core::config::config_home(), home);
    assert_eq!(dirs::home_dir().unwrap(), home);
    let content = match case.as_str() {
        "two" => format!("{}étail", "a".repeat(1999)),
        "three" => format!("{}界tail", "a".repeat(1999)),
        "four" => format!("{}\u{10000}tail", "a".repeat(1998)),
        "ascii" => "a".repeat(2010),
        "exact" => format!("{}界", "a".repeat(1997)),
        "short" => "short 界".to_string(),
        _ => panic!("unknown case"),
    };
    if matches!(case.as_str(), "two" | "three" | "four") {
        assert!(!content.is_char_boundary(2000));
    }
    let files = [
        (
            "project (.coven-code/AGENTS.md)",
            project.join(".coven-code/AGENTS.md"),
        ),
        ("project (AGENTS.md)", project.join("AGENTS.md")),
        (
            "global (~/.coven-code/AGENTS.md)",
            home.join(".coven-code/AGENTS.md"),
        ),
    ];
    for (_, path) in &files {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &content).unwrap();
    }
    let mut context = CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: vec![],
        working_dir: project,
        session_id: "fixture-session".into(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(MemoryCommand.execute("", &mut context));
    let CommandResult::Message(message) = result else {
        panic!("unexpected result: {result:?}")
    };
    let display = if content.len() > 2000 {
        let mut prefix = String::new();
        for character in content.chars() {
            if prefix.len() + character.len_utf8() > 2000 {
                break;
            }
            prefix.push(character);
        }
        format!("{}…\n(truncated — file is {} chars)", prefix, content.len())
    } else {
        content.clone()
    };
    for (label, path) in &files {
        let section = format!("\n[{label}]\nPath: {}\nSize: 1 lines, {} chars\n─────────────────────────────────\n{display}\n", path.display(), content.len());
        assert!(
            message.contains(&section),
            "missing expected section for {label}"
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), content);
    }
    assert_eq!(
        message.matches("(truncated — file is").count(),
        if content.len() > 2000 { 3 } else { 0 }
    );
    assert!(message.contains("Subcommands:"));
    std::fs::write(std::env::var_os("MEMORY_FIXTURE_DONE").unwrap(), b"checked").unwrap();
}
