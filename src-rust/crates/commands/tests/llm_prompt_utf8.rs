use claurst_commands::{CommandContext, CommandResult, RenameCommand, ReviewCommand, SlashCommand};
use claurst_core::{
    config::{Config, ProviderConfig},
    cost::CostTracker,
    types::Message,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn review_prompt_preserves_utf8() {
    run_cases("review");
}

#[test]
fn rename_prompt_preserves_utf8() {
    run_cases("rename");
}

fn run_cases(operation: &str) {
    let mut failures = Vec::new();
    for case in ["two", "three", "four", "ascii", "exact", "short"] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&project).unwrap();
        let log = temp.path().join("stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "prompt_worker", "--ignored", "--nocapture"])
            .env_clear()
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("COVEN_CODE_HOME", &home)
            .env("COVEN_HOME", temp.path().join("coven"))
            .env("COVEN_CAVE_HOME", temp.path().join("cave"))
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("NO_PROXY", "*")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join("nonexistent-gitconfig"))
            .env("PROMPT_OPERATION", operation)
            .env("PROMPT_CASE", case)
            .env("PROMPT_PROJECT", &project)
            .current_dir(&project)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
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
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn git(project: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(project)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn prefix(text: &str, budget: usize) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        if result.len() + ch.len_utf8() > budget {
            break;
        }
        result.push(ch);
    }
    result
}

// Capture the actual provider request, then return a non-retryable error so
// neither command can reach success-only publication or session mutations.
fn capture_request(listener: TcpListener) -> serde_json::Value {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut socket = loop {
        match listener.accept() {
            Ok((socket, peer)) => {
                assert!(peer.ip().is_loopback());
                break socket;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(e) => panic!("fixture accept: {e}"),
        }
    };
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut data = Vec::new();
    let mut chunk = [0; 4096];
    let header_end = loop {
        if let Some(index) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break index + 4;
        }
        let count = socket.read(&mut chunk).unwrap();
        assert!(count > 0);
        data.extend_from_slice(&chunk[..count]);
        assert!(data.len() < 200_000);
    };
    let headers = std::str::from_utf8(&data[..header_end]).unwrap();
    assert!(headers.starts_with("POST /v1/messages HTTP/1.1\r\n"));
    assert!(headers
        .to_ascii_lowercase()
        .contains("x-api-key: fixture-key\r\n"));
    let length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap();
    assert!(length < 200_000);
    while data.len() < header_end + length {
        let count = socket.read(&mut chunk).unwrap();
        assert!(count > 0);
        data.extend_from_slice(&chunk[..count]);
    }
    let request = serde_json::from_slice(&data[header_end..header_end + length]).unwrap();
    let body =
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"fixture-stop"}}"#;
    write!(socket, "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    request
}

#[test]
#[ignore = "isolated homes and loopback provider"]
fn prompt_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("PROMPT_PROJECT").unwrap());
    assert_eq!(claurst_core::config::config_home(), home);
    assert_eq!(dirs::home_dir().unwrap(), home);
    let operation = std::env::var("PROMPT_OPERATION").unwrap();
    let case = std::env::var("PROMPT_CASE").unwrap();
    let (input, messages, budget) = if operation == "review" {
        git(&project, &["init", "--quiet"]);
        git(&project, &["config", "core.autocrlf", "false"]);
        git(&project, &["config", "color.ui", "false"]);
        let file = project.join("fixture.txt");
        std::fs::write(&file, "").unwrap();
        git(&project, &["add", "-N", "fixture.txt"]);
        assert!(git(&project, &["diff", "--cached"]).is_empty());
        std::fs::write(&file, "a\n").unwrap();
        let probe = String::from_utf8(git(&project, &["diff"])).unwrap();
        let start = probe.find("\n+a\n").unwrap() + 2;
        let payload = match case.as_str() {
            "two" => format!("{}étail\n", "a".repeat(99_999 - start)),
            "three" => format!("{}界tail\n", "a".repeat(99_999 - start)),
            "four" => format!("{}\u{10000}tail\n", "a".repeat(99_998 - start)),
            "ascii" => format!("{}\n", "a".repeat(100_100)),
            "exact" => format!("{}界\n", "a".repeat(99_997 - start)),
            "short" => "short 界\n".into(),
            _ => panic!("unknown case"),
        };
        std::fs::write(&file, payload).unwrap();
        (
            String::from_utf8(git(&project, &["diff"]))
                .unwrap()
                .trim()
                .to_owned(),
            vec![],
            100_000,
        )
    } else {
        let mut texts = vec!["a".repeat(320); 6];
        let beginning = (0..6)
            .map(|_| format!("User: {}", "a".repeat(300)))
            .collect::<Vec<_>>()
            .join("\n")
            + "\nUser: ";
        let remaining = 2000 - beginning.len();
        let last = match case.as_str() {
            "two" => format!("{}étail", "a".repeat(remaining - 1)),
            "three" => format!("{}界tail", "a".repeat(remaining - 1)),
            "four" => format!("{}\u{10000}tail", "a".repeat(remaining - 2)),
            "ascii" => "a".repeat(remaining + 20),
            "exact" => format!("{}界", "a".repeat(remaining - 3)),
            "short" => {
                texts.clear();
                "short 界".into()
            }
            _ => panic!("unknown case"),
        };
        texts.push(last);
        let excerpt = texts
            .iter()
            .map(|text| format!("User: {}", text.chars().take(300).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n");
        (
            excerpt,
            texts.into_iter().map(Message::user).collect(),
            2000,
        )
    };
    if matches!(case.as_str(), "two" | "three" | "four") {
        assert!(input.len() > budget);
        assert!(!input.is_char_boundary(budget));
    }
    if case == "exact" {
        assert_eq!(input.len(), budget);
    }
    let original_file =
        (operation == "review").then(|| std::fs::read(project.join("fixture.txt")).unwrap());
    let original_messages = serde_json::to_vec(&messages).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || capture_request(listener));
    let mut config = Config {
        api_key: Some("fixture-key".into()),
        provider: Some("anthropic".into()),
        ..Config::default()
    };
    config.provider_configs.insert(
        "anthropic".into(),
        ProviderConfig {
            api_base: Some(format!("http://{address}")),
            ..ProviderConfig::default()
        },
    );
    assert_eq!(
        config.resolve_anthropic_api_base(),
        format!("http://{address}")
    );
    let mut context = CommandContext {
        config,
        cost_tracker: CostTracker::new(),
        messages,
        working_dir: project.clone(),
        session_id: "fixture-session".into(),
        session_title: Some("unchanged-title".into()),
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            if operation == "review" {
                ReviewCommand.execute("", &mut context).await
            } else {
                RenameCommand.execute("", &mut context).await
            }
        })
        .await
        .unwrap()
    });
    let CommandResult::Error(error) = result else {
        panic!("{result:?}");
    };
    assert!(error.contains("fixture-stop"), "{error}");
    let request = server.join().unwrap();
    assert_eq!(request["messages"].as_array().unwrap().len(), 1);
    assert_eq!(request["messages"][0]["role"], "user");
    let prompt = request["messages"][0]["content"].as_str().unwrap();
    let expected = prefix(&input, budget);
    if operation == "review" {
        let (_, diff) = prompt.split_once("```diff\n").unwrap();
        let suffix = if input.len() > budget {
            "\n\n[... diff truncated at 100000 chars ...]"
        } else {
            ""
        };
        assert_eq!(diff, format!("{expected}{suffix}\n```"));
        assert!(prompt.contains("Changed files (1):\n  - fixture.txt"));
        assert_eq!(
            std::fs::read(project.join("fixture.txt")).unwrap(),
            original_file.unwrap()
        );
        assert_eq!(git(&project, &["show", ":fixture.txt"]), b"");
    } else {
        assert_eq!(prompt, format!("Conversation to name:\n\n{expected}"));
    }
    assert_eq!(
        serde_json::to_vec(&context.messages).unwrap(),
        original_messages
    );
    assert_eq!(context.session_title.as_deref(), Some("unchanged-title"));
    assert_eq!(context.session_id, "fixture-session");
    std::fs::write(home.join("done"), b"checked").unwrap();
}
