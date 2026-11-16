//! Real engine processes and real tools against a deterministic loopback model.
//! No credentials, provider CLIs, installed state, or inference are used.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

struct Fixture {
    root: tempfile::TempDir,
    requests: Arc<Mutex<Vec<Value>>>,
    address: std::net::SocketAddr,
    done: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = done.clone();
        let worker = std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => serve(stream, &received),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            }
        });
        Self {
            root,
            requests,
            address,
            done,
            worker: Some(worker),
        }
    }

    fn engine(&self, format: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_coven-code"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", self.root.path().join("home"))
            .env("COVEN_HOME", self.root.path().join("coven"))
            .env("COVEN_CODE_DISABLE_MODELS_FETCH", "1")
            .env("COVEN_CODE_API_BASE", format!("http://{}", self.address))
            .env("ANTHROPIC_API_KEY", "local-fixture-only")
            .current_dir(self.root.path())
            .args([
                "--print",
                "--output-format",
                format,
                "--permission-mode",
                "bypass-permissions",
                "--model",
                "claude-fixture",
            ]);
        command
    }

    fn run(&self, args: &[&str], stream_input: bool) -> Output {
        self.run_format(args, stream_input, "stream-json")
    }

    fn run_format(&self, args: &[&str], stream_input: bool, format: &str) -> Output {
        let mut command = self.engine(format);
        command.args(args);
        if stream_input {
            command.args(["--input-format", "stream-json"]);
        } else {
            command.arg("Finish the unfinished numbered work.");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if stream_input {
            writeln!(child.stdin.take().unwrap(), "{}", json!({"type":"user","message":{"role":"user","content":"Finish the unfinished numbered work."}})).unwrap();
        } else {
            drop(child.stdin.take());
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                child.kill().unwrap();
                panic!("engine timed out: {:?}", child.wait_with_output().unwrap());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        if let Some(directory) = std::env::var_os("COVEN_CODE_RUN_BUDGET_RECEIPTS") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let ledger = std::fs::read_to_string(self.root.path().join("ledger")).ok();
            std::fs::write(directory.join(format!("{}.json", uuid::Uuid::new_v4())), serde_json::to_vec_pretty(&json!({
                "args":args,"stream_input":stream_input,"format":format,
                "exit_code":output.status.code(),"stdout":String::from_utf8_lossy(&output.stdout),
                "stderr":String::from_utf8_lossy(&output.stderr),"requests":*self.requests.lock().unwrap(),"ledger":ledger,
            })).unwrap()).unwrap();
        }
        output
    }

    fn ledger(&self) -> String {
        std::fs::read_to_string(self.root.path().join("ledger")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.done.store(true, std::sync::atomic::Ordering::Relaxed);
        let result = self.worker.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

fn serve(stream: TcpStream, requests: &Mutex<Vec<Value>>) {
    // Darwin can inherit the listener nonblocking flag on accepted sockets.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap();
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let request: Value = serde_json::from_slice(&body).unwrap();
    let completed = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .filter(|block| block["type"] == "tool_result")
        .count();
    for message in request["messages"].as_array().unwrap() {
        if let Some(blocks) = message["content"].as_array() {
            for block in blocks.iter().filter(|block| block["type"] == "tool_result") {
                assert_ne!(block["is_error"], true, "fixture tool failed: {block}");
            }
        }
    }
    let authentication_failure = request.to_string().contains("authentication-fixture");
    requests.lock().unwrap().push(request);
    if authentication_failure {
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"fixture credential refused"}}"#;
        write!(reader.into_inner(), "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        return;
    }
    let blocks = if completed < 10 {
        vec![
            json!({"type":"text","text":format!("Partial round {}. ", completed + 1)}),
            json!({"type":"tool_use","id":format!("step-{}", completed + 1),"name":"Bash","input":{"command":format!("printf '{}\\n' >> ledger",completed + 1)}}),
        ]
    } else {
        vec![json!({"type":"text","text":"All ten side effects are complete."})]
    };
    let mut events = vec![
        json!({"type":"message_start","message":{"id":"fixture","type":"message","role":"assistant","model":"claude-fixture","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}),
    ];
    for (index, block) in blocks.into_iter().enumerate() {
        if block["type"] == "text" {
            events.push(json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}}));
            events.push(json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":block["text"]}}));
        } else {
            events.push(json!({"type":"content_block_start","index":index,"content_block":block}));
        }
        events.push(json!({"type":"content_block_stop","index":index}));
    }
    events.push(json!({"type":"message_delta","delta":{"stop_reason":if completed < 10 {"tool_use"}else{"end_turn"}},"usage":{"output_tokens":1}}));
    events.push(json!({"type":"message_stop"}));
    let body: String = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect();
    let mut stream = reader.into_inner();
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
}

fn result(output: &Output) -> Value {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|frame| frame["type"] == "result")
        .unwrap_or_else(|| panic!("missing result: {output:?}"))
}

#[test]
fn budget_twelve_completes_eleven_real_model_rounds() {
    for stream in [false, true] {
        let fixture = Fixture::new();
        let output = fixture.run(&["--max-turns", "12"], stream);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(fixture.requests.lock().unwrap().len(), 11, "{output:?}");
        assert_eq!(
            result(&output)["turn_budget"],
            json!({"max_turns":12,"source":"explicit","consumed":11})
        );
        assert_eq!(
            fixture.ledger(),
            (1..=10).map(|n| format!("{n}\n")).collect::<String>()
        );
    }
}

#[test]
fn budget_ten_is_typed_and_resumes_saved_work_without_repeating_writes() {
    for stream in [false, true] {
        let fixture = Fixture::new();
        let first = fixture.run(
            &["--max-turns", "10", "--session-id", "budget-continuation"],
            stream,
        );
        assert_eq!(fixture.requests.lock().unwrap().len(), 10, "{first:?}");
        let terminal = result(&first);
        assert_eq!(terminal["outcome"], "turn_budget_exhausted", "{first:?}");
        assert_eq!(terminal["subtype"], "error_during_execution");
        assert_eq!(terminal["is_error"], true);
        assert_eq!(terminal["num_turns"], 10);
        assert_eq!(
            terminal["turn_budget"],
            json!({"max_turns":10,"source":"explicit","consumed":10})
        );
        assert_eq!(
            terminal["continuation"],
            json!({"session_id":"budget-continuation","resumable":true})
        );
        assert_eq!(first.status.code(), Some(if stream { 0 } else { 1 }));
        let before = fixture.ledger();
        let second = fixture.run(
            &["--resume", "budget-continuation", "--max-turns", "12"],
            stream,
        );
        assert!(second.status.success(), "{second:?}");
        assert_eq!(fixture.requests.lock().unwrap().len(), 11, "{second:?}");
        assert_eq!(
            fixture.ledger(),
            before,
            "resume repeated a completed filesystem side effect"
        );
        assert_eq!(result(&second)["session_id"], "budget-continuation");
        let requests = fixture.requests.lock().unwrap();
        let restored = requests.last().unwrap().to_string();
        for n in 1..=10 {
            assert!(
                restored.contains(&format!("step-{n}")),
                "missing tool result {n}"
            );
        }
        assert!(
            restored.contains("Partial round 1."),
            "partial output lost at restart"
        );
    }
}

#[test]
fn invalid_budgets_fail_before_model_invocation() {
    for invalid in ["0", "-1", "unlimited", "4294967296"] {
        let fixture = Fixture::new();
        let output = fixture.run(&["--max-turns", invalid], false);
        assert_eq!(output.status.code(), Some(2), "{invalid}: {output:?}");
        assert!(fixture.requests.lock().unwrap().is_empty());
    }
}

#[test]
fn default_and_agent_budget_are_disclosed_and_explicit_selection_wins() {
    for stream in [false, true] {
        for (agent_limit, explicit, expected, source) in [
            (None, None, 10, "default"),
            (Some(3), None, 3, "agent"),
            (Some(3), Some("12"), 11, "explicit"),
            (Some(30), Some("10"), 10, "explicit"),
        ] {
            let fixture = Fixture::new();
            std::fs::create_dir(fixture.root.path().join(".coven-code")).unwrap();
            std::fs::write(
                fixture.root.path().join(".coven-code/settings.json"),
                json!({"agents":{"fixture":{"max_turns":agent_limit,"access":"full"}}}).to_string(),
            )
            .unwrap();
            let mut args = vec!["--agent", "fixture"];
            if let Some(limit) = explicit {
                args.extend(["--max-turns", limit]);
            }
            let output = fixture.run(&args, stream);
            assert_eq!(
                fixture.requests.lock().unwrap().len(),
                expected,
                "{output:?}"
            );
            assert_eq!(
                result(&output)["turn_budget"]["source"],
                source,
                "{output:?}"
            );
        }
    }
}

#[test]
fn missing_resume_and_failed_persistence_never_promise_continuation() {
    for stream in [false, true] {
        let fixture = Fixture::new();
        let output = fixture.run(&["--resume", "missing"], stream);
        assert!(!output.status.success(), "{output:?}");
        assert!(fixture.requests.lock().unwrap().is_empty());
        std::fs::create_dir_all(fixture.root.path().join("coven/code")).unwrap();
        std::fs::write(
            fixture.root.path().join("coven/code/sessions"),
            "save obstruction",
        )
        .unwrap();
        let output = fixture.run(&["--max-turns", "10"], stream);
        assert_eq!(result(&output)["outcome"], "turn_budget_exhausted");
        assert_eq!(result(&output)["continuation"]["resumable"], false);
        assert!(String::from_utf8_lossy(&output.stderr).contains("failed to persist session"));
    }
}

#[test]
fn capability_detection_is_offline_and_does_not_create_state() {
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_coven-code"))
        .env_clear()
        .env("HOME", fixture.root.path().join("home"))
        .env("COVEN_HOME", fixture.root.path().join("coven"))
        .env("COVEN_PARENT", "coven")
        .arg("--run-budget-capabilities")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let capabilities: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(capabilities["engine_contract_version"], 1);
    assert_eq!(capabilities["run_budget_version"], 1);
    assert_eq!(capabilities["flag"], "--max-turns");
    assert!(!fixture.root.path().join("coven").exists());
    assert!(!fixture.root.path().join("home").exists());
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[test]
fn one_shot_text_and_json_preserve_partial_work_on_exhaustion() {
    for format in ["text", "json"] {
        let fixture = Fixture::new();
        let output = fixture.run_format(
            &["--max-turns", "10", "--session-id", "format-budget"],
            false,
            format,
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        if format == "json" {
            assert_eq!(result(&output)["outcome"], "turn_budget_exhausted");
            assert!(result(&output)["result"]
                .as_str()
                .unwrap()
                .contains("Partial round 1."));
        } else {
            assert!(String::from_utf8_lossy(&output.stdout).contains("Partial round 1."));
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains("Model-round budget exhausted (10/10)"));
        }
        let saved = std::fs::read_to_string(
            fixture
                .root
                .path()
                .join("coven/code/sessions/format-budget.json"),
        )
        .unwrap();
        assert!(saved.contains("Partial round 1.") && saved.contains("step-10"));
        assert_eq!(fixture.requests.lock().unwrap().len(), 10);
    }
}

#[test]
fn authentication_errors_remain_distinct_from_local_exhaustion() {
    for stream in [false, true] {
        let fixture = Fixture::new();
        let output = fixture.run(
            &[
                "--max-turns",
                "10",
                "--system-prompt",
                "authentication-fixture",
            ],
            stream,
        );
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        let wire = if stream {
            result(&output).to_string()
        } else {
            String::from_utf8_lossy(&output.stderr).into_owned()
        };
        assert!(!wire.contains("turn_budget_exhausted"), "{output:?}");
        assert!(wire.contains("fixture credential refused"), "{output:?}");
        assert_eq!(output.status.code(), Some(if stream { 0 } else { 1 }));
    }
}

#[test]
fn resumed_session_refuses_wrong_directory_and_identity_before_work() {
    let fixture = Fixture::new();
    let first = fixture.run(
        &["--session-id", "resume-integrity", "--max-turns", "10"],
        true,
    );
    assert_eq!(result(&first)["outcome"], "turn_budget_exhausted");
    let other = fixture.root.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let wrong_cwd = fixture.run(
        &[
            "--resume",
            "resume-integrity",
            "--cwd",
            other.to_str().unwrap(),
        ],
        true,
    );
    assert!(!wrong_cwd.status.success());
    assert!(String::from_utf8_lossy(&wrong_cwd.stderr).contains("Resume working directory differs"));
    let saved = fixture
        .root
        .path()
        .join("coven/code/sessions/resume-integrity.json");
    let mut session: Value =
        serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    session["id"] = "wrong-session".into();
    std::fs::write(saved, session.to_string()).unwrap();
    let wrong_id = fixture.run(&["--resume", "resume-integrity"], false);
    assert!(!wrong_id.status.success());
    assert!(String::from_utf8_lossy(&wrong_id.stderr).contains("Saved session identity disagrees"));
    assert_eq!(fixture.requests.lock().unwrap().len(), 10);
}
