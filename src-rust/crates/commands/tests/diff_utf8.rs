use claurst_commands::{CommandContext, CommandResult, DiffCommand, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn diff_preserves_utf8_at_byte_limit() {
    run_cases(&["two", "three", "four"]);
}
#[test]
fn diff_ascii_and_short_controls() {
    run_cases(&["ascii", "short"]);
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
            .args(["--exact", "diff_utf8_worker", "--ignored", "--nocapture"])
            .env_clear()
            .env("COVEN_CODE_HOME", &home)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join("nonexistent-gitconfig"))
            .env("DIFF_FIXTURE_PROJECT", &project)
            .env("DIFF_FIXTURE_CASE", case)
            .env("DIFF_FIXTURE_DONE", temp.path().join("done"))
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
#[ignore = "owned configuration and real git repository"]
fn diff_utf8_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("DIFF_FIXTURE_PROJECT").unwrap());
    let case = std::env::var("DIFF_FIXTURE_CASE").unwrap();
    assert_eq!(claurst_core::config::config_home(), home);
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&project)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    git(&["init", "--quiet"]);
    git(&["config", "core.autocrlf", "false"]);
    git(&["config", "color.ui", "false"]);
    let file = project.join("fixture.txt");
    std::fs::write(&file, "").unwrap();
    git(&["add", "fixture.txt"]);
    std::fs::write(&file, "a\n").unwrap();
    let probe = String::from_utf8(git(&["diff"])).unwrap();
    let start = probe.find("\n+a\n").unwrap() + 2;
    let payload = match case.as_str() {
        "two" => format!("{}étail\n", "a".repeat(7999 - start)),
        "three" => format!("{}界tail\n", "a".repeat(7999 - start)),
        "four" => format!("{}\u{10000}tail\n", "a".repeat(7998 - start)),
        "ascii" => format!("{}\n", "a".repeat(8100)),
        "short" => "short 界\n".to_string(),
        _ => panic!("unknown case"),
    };
    std::fs::write(&file, &payload).unwrap();
    let raw = String::from_utf8(git(&["diff"])).unwrap();
    if matches!(case.as_str(), "two" | "three" | "four") {
        assert!(raw.len() > 8000);
        assert!(
            !raw.is_char_boundary(8000),
            "fixture did not split a character"
        );
    }
    let mut context = CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: vec![],
        working_dir: project.clone(),
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
    let result = runtime.block_on(DiffCommand.execute("", &mut context));
    let CommandResult::Message(message) = result else {
        panic!("unexpected result: {result:?}")
    };
    let expected = if raw.len() > 8000 {
        let mut prefix = String::new();
        for character in raw.chars() {
            if prefix.len() + character.len_utf8() > 8000 {
                break;
            }
            prefix.push(character);
        }
        format!(
            "Changes:\n{}\n… (truncated — {} total bytes; use `git diff` for full output)",
            prefix,
            raw.len()
        )
    } else {
        format!("Changes:\n{raw}")
    };
    assert_eq!(message, expected);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), payload);
    assert_eq!(git(&["show", ":fixture.txt"]), b"");
    std::fs::write(std::env::var_os("DIFF_FIXTURE_DONE").unwrap(), b"checked").unwrap();
}
