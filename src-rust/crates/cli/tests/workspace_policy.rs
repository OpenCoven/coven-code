//! Exercise the shipped entrypoint and native sandbox without provider credentials.
#![cfg(target_os = "macos")]

use nix::{
    sys::signal::{kill, killpg, Signal},
    unistd::{getpgid, getuid, Pid},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::unix::process::{CommandExt, ExitStatusExt},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[test]
fn installed_entrypoint_reports_exact_kernel_scope_and_rejects_a_changed_root() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("a-work");
    let project = base.join("b-project");
    let read = base.join("c-read");
    let file = base.join("policy.json");
    for root in [&work, &project, &read] {
        fs::create_dir(root).unwrap();
    }
    let policy = json!({
        "version": 1, "runId": "run-native-test", "access": "workspace-write",
        "primaryRoot": work, "writableRoots": [work, project], "readOnlyRoots": [read],
    });
    fs::write(&file, policy.to_string()).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_coven-code"))
            .arg("--verify-workspace-policy-v1")
            .arg(&file)
            .env("COVEN_HOME", base.join("coven"))
            .env("COVEN_CAVE_HOME", base.join("cave"))
            .current_dir(&work)
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success(), "{:?}", output);
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut expected = policy;
    expected["enforced"] = json!(true);
    assert_eq!(actual, expected);
    fs::rename(&project, base.join("changed")).unwrap();
    let output = run();
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

#[test]
fn scoped_turn_does_not_delegate_to_an_unqualified_claude_cli() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("workspace");
    let file = base.join("policy.json");
    let claude = base.join("claude");
    let marker = base.join("launched");
    fs::create_dir(&work).unwrap();
    fs::write(
        &file,
        json!({"version":1,"runId":"no-delegation","access":"workspace-write",
            "primaryRoot":work,"writableRoots":[work],"readOnlyRoots":[]})
        .to_string(),
    )
    .unwrap();
    fs::write(
        &claude,
        "#!/bin/sh\nprintf started > \"$SCOPED_TEST_MARKER\"\n",
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_coven-code"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &base)
        .env("COVEN_HOME", base.join("coven"))
        .env("COVEN_CAVE_HOME", base.join("cave"))
        .env("COVEN_CODE_HOME", base.join("code"))
        .env("COVEN_CODE_CLAUDE_BIN", &claude)
        .env("SCOPED_TEST_MARKER", &marker)
        .current_dir(&work)
        .args([
            "--print",
            "--output-format",
            "stream-json",
            "--provider",
            "anthropic",
        ])
        .arg("--chat-workspace-policy-v1")
        .arg(&file)
        .args(["--", "Fixture turn"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("requires direct Anthropic credentials"),
        "{output:?}"
    );
    assert!(!marker.exists());
}

fn scope(run_id: &str, work: &Path, writable: &[&Path], read_only: &[&Path]) -> Value {
    json!({
        "version": 1, "runId": run_id, "access": "workspace-write",
        "primaryRoot": work, "writableRoots": writable, "readOnlyRoots": read_only,
    })
}

/// The admission the engine must report for `policy` before any model work.
fn receipt(policy: &Value) -> Value {
    let mut receipt = policy.clone();
    receipt["enforced"] = json!(true);
    receipt
}

/// The scope a run reported. Coven accepts it only as the run's first frame
/// (`normalize_native_stream_line` in coven-cli's pty_runner.rs).
fn admission(frames: &[Value]) -> Option<&Value> {
    frames
        .first()
        .and_then(|frame| frame.get("effectiveAccess"))
}

/// A loopback stand-in for the Messages API. Each connection is answered
/// with the next scripted reply, and its request line is sent to the test.
/// A thread whose replies a correct run never asks for stays blocked in
/// `accept` until the test binary exits.
fn messages_fixture(replies: Vec<String>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (requests, received) = mpsc::channel();
    // A stopped run may drop its connection part way; the thread then ends.
    thread::spawn(move || {
        for reply in replies {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            let Ok(clone) = socket.try_clone() else {
                return;
            };
            let mut reader = BufReader::new(clone);
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                return;
            }
            let mut length = 0;
            loop {
                let mut header = String::new();
                match reader.read_line(&mut header) {
                    Ok(0) | Err(_) => return,
                    Ok(_) if header == "\r\n" => break,
                    Ok(_) => {}
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        let Ok(value) = value.trim().parse() else {
                            return;
                        };
                        length = value;
                    }
                }
            }
            if reader.read_exact(&mut vec![0; length]).is_err() {
                return;
            }
            let _ = requests.send(request);
            let _ = write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    (base, received)
}

/// One streamed model reply holding a single content block, in the wire
/// shape of the Messages API.
fn model_reply(block: Value, delta: Value, stop_reason: &str) -> String {
    [
        json!({"type": "message_start", "message": {
            "id": "fixture", "type": "message", "role": "assistant", "content": [],
            "model": "fixture", "usage": {"input_tokens": 1, "output_tokens": 0},
        }}),
        json!({"type": "content_block_start", "index": 0, "content_block": block}),
        json!({"type": "content_block_delta", "index": 0, "delta": delta}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": stop_reason},
            "usage": {"output_tokens": 1}}),
        json!({"type": "message_stop"}),
    ]
    .into_iter()
    .map(|data| {
        format!(
            "event: {}\ndata: {data}\n\n",
            data["type"].as_str().unwrap()
        )
    })
    .collect()
}

/// A model reply calling the tool `name` with the call id `id`.
fn tool_call(id: &str, name: &str, input: Value) -> String {
    model_reply(
        json!({"type": "tool_use", "id": id, "name": name, "input": {}}),
        json!({"type": "input_json_delta", "partial_json": input.to_string()}),
        "tool_use",
    )
}

fn final_text(text: &str) -> String {
    model_reply(
        json!({"type": "text", "text": ""}),
        json!({"type": "text_delta", "text": text}),
        "end_turn",
    )
}

fn user_frame(text: &str) -> Vec<u8> {
    format!(
        "{}\n",
        json!({"type": "user", "message": {"role": "user", "content": text}})
    )
    .into_bytes()
}

/// A scoped engine run. If it is dropped before exiting, for example by a
/// failed assertion, its whole process group is killed.
struct Engine(Child);

impl Drop for Engine {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            let _ = killpg(group_of(&self.0), Signal::SIGKILL);
            let _ = self.0.wait();
        }
    }
}

/// Launch a scoped Chat stream with stream-json in and out, leading a process
/// group of its own, which is what Coven's Stop signals (Coven also starts a
/// new session). `log` names the run's captured stdout and stderr.
fn scoped_stream(base: &Path, work: &Path, policy: &Path, api: &str, log: &str) -> Engine {
    let capture = |stream: &str| fs::File::create(base.join(format!("{log}.{stream}"))).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_coven-code"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", base)
        .env("COVEN_HOME", base.join("coven"))
        .env("COVEN_CAVE_HOME", base.join("cave"))
        .env("COVEN_CODE_HOME", base.join("code"))
        .env("ANTHROPIC_API_KEY", "local-fixture")
        .env("ANTHROPIC_BASE_URL", api)
        .current_dir(work)
        .args([
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--provider",
            "anthropic",
            "--chat-workspace-policy-v1",
        ])
        .arg(policy)
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(capture("stdout"))
        .stderr(capture("stderr"))
        .spawn()
        .unwrap();
    Engine(child)
}

/// The run's stdout frames. Like Coven, skip blank lines and fail on any other
/// line that is not JSON. Only a run killed mid-write can leave an unfinished
/// last line, which is dropped.
fn frames(base: &Path, log: &str) -> Vec<Value> {
    let stdout = fs::read_to_string(base.join(format!("{log}.stdout"))).unwrap();
    let finished = stdout.rfind('\n').map_or(0, |end| end + 1);
    stdout[..finished]
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|error| {
                panic!(
                    "the run wrote a line that is not JSON ({error}): {line}\n{}",
                    logs(base, log)
                )
            })
        })
        .collect()
}

/// The engine's result frame for the fixture's tool call `id`.
fn tool_result<'a>(frames: &'a [Value], id: &str) -> &'a Value {
    frames
        .iter()
        .find(|frame| frame["type"] == "tool_result" && frame["tool_use_id"] == id)
        .unwrap_or(&Value::Null)
}

fn logs(base: &Path, log: &str) -> String {
    ["stdout", "stderr"]
        .map(|stream| {
            let text = fs::read_to_string(base.join(format!("{log}.{stream}")));
            format!("--- {log}.{stream}\n{}", text.unwrap_or_default())
        })
        .join("\n")
}

fn group_of(engine: &Child) -> Pid {
    Pid::from_raw(i32::try_from(engine.id()).unwrap())
}

/// Wait for the run to exit by itself; kill its whole group if it does not.
fn exit_within(engine: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Some(status) = engine.try_wait().unwrap() {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = killpg(group_of(engine), Signal::SIGKILL);
    let _ = engine.wait();
    None
}

/// Kill a shell that escaped, so it cannot write into the deleted tempdir,
/// and fail the test with the run's logs.
fn kill_and_fail(shell: Pid, why: &str, base: &Path, log: &str) -> ! {
    let _ = killpg(shell, Signal::SIGKILL);
    let _ = kill(shell, Signal::SIGKILL);
    panic!("{why}\n{}", logs(base, log))
}

/// Whether `shell` is gone within `limit`.
fn reaped_within(shell: Pid, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while kill(shell, None).is_ok() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    true
}

/// Stop a scoped run while a shell command sleeps between its two writes, and
/// check that the shell is reaped and its second write never lands. With
/// `background`, the delayed command runs as a background task, and the
/// fixture's next reply keeps the run busy with a long foreground command, so
/// the run cannot finish before the stop.
fn stop_mid_command(background: bool) {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("workspace");
    fs::create_dir(&work).unwrap();
    let policy = base.join("policy.json");
    let admitted = scope("stop-mid-command", &work, &[&work], &[]);
    fs::write(&policy, admitted.to_string()).unwrap();
    let (started, late) = (work.join("started"), work.join("late"));
    // The shell records its own PID, then waits before its second write.
    let delayed = format!(
        "printf %s $$ > '{}'; sleep 5; printf late > '{}'",
        started.display(),
        late.display()
    );
    let replies = if background {
        vec![
            tool_call(
                "delayed",
                "Bash",
                json!({"command": delayed, "run_in_background": true}),
            ),
            tool_call("hold", "Bash", json!({"command": "sleep 30"})),
        ]
    } else {
        vec![tool_call("delayed", "Bash", json!({"command": delayed}))]
    };
    let (api, _requests) = messages_fixture(replies);
    let mut engine = scoped_stream(&base, &work, &policy, &api, "stop");
    let stdin = engine.0.stdin.as_mut().unwrap();
    stdin
        .write_all(&user_frame("Run the delayed write"))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let shell = loop {
        let pid = fs::read_to_string(&started)
            .ok()
            .and_then(|pid| pid.parse().ok());
        if let Some(pid) = pid {
            break Pid::from_raw(pid);
        }
        let running = engine.0.try_wait().unwrap().is_none();
        assert!(
            running && Instant::now() < deadline,
            "the scoped shell never started\n{}",
            logs(&base, "stop")
        );
        thread::sleep(Duration::from_millis(10));
    };
    let stopped_at = Instant::now();
    assert!(
        !late.exists(),
        "the stop came after the command's own delay; the test runner stalled"
    );
    // Coven stops the run's process group, so the shell must still be in it.
    match getpgid(Some(shell)) {
        Ok(group) if group == group_of(&engine.0) => {}
        Ok(group) => kill_and_fail(
            shell,
            &format!("the scoped shell left the run's process group for {group}"),
            &base,
            "stop",
        ),
        Err(error) => panic!(
            "the scoped shell exited before the stop: {error}\n{}",
            logs(&base, "stop")
        ),
    }
    // Coven's Stop for a harness run: one SIGKILL to the run's process group
    // (`terminate_unix_process_group` in coven-cli's pty_runner.rs).
    killpg(group_of(&engine.0), Signal::SIGKILL)
        .unwrap_or_else(|error| panic!("the stop failed: {error}\n{}", logs(&base, "stop")));
    let status = engine.0.wait().unwrap();
    assert_eq!(
        status.signal(),
        Some(Signal::SIGKILL as i32),
        "{status:?}\n{}",
        logs(&base, "stop")
    );
    // The shell was inside the stopped group, so it is gone once reaped.
    if !reaped_within(shell, Duration::from_secs(3)) {
        kill_and_fail(shell, "the scoped shell outlived the stop", &base, "stop");
    }
    // The shell is gone, so nothing should write now. Outlast the command's
    // own delay anyway before looking for its write.
    thread::sleep((stopped_at + Duration::from_secs(6)).saturating_duration_since(Instant::now()));
    assert!(
        !late.exists(),
        "a stopped scoped run still wrote after its delay\n{}",
        logs(&base, "stop")
    );
    let frames = frames(&base, "stop");
    assert_eq!(
        admission(&frames),
        Some(&receipt(&admitted)),
        "the run was not admitted with the exact scope\n{}",
        logs(&base, "stop")
    );
    assert!(
        !frames.iter().any(|frame| frame["type"] == "result"),
        "the run finished before the stop\n{}",
        logs(&base, "stop")
    );
}

#[test]
fn stopping_a_scoped_run_mid_command_prevents_its_delayed_write() {
    stop_mid_command(false);
}

#[test]
fn stopping_a_scoped_run_prevents_a_delayed_background_write() {
    stop_mid_command(true);
}

#[test]
fn a_downgraded_scope_governs_the_next_reply() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("a-work");
    let project = base.join("b-project");
    for root in [&work, &project] {
        fs::create_dir(root).unwrap();
    }
    let policy = base.join("policy.json");
    // Each reply is a new scoped launch under the host's current policy. The
    // model runs one shell command, writes one file, then answers.
    let reply = |log: &str, admitted: &Value, command: &str, file: &Path, content: &str| {
        fs::write(&policy, admitted.to_string()).unwrap();
        let (api, _requests) = messages_fixture(vec![
            tool_call("shell", "Bash", json!({"command": command})),
            tool_call(
                "write",
                "Write",
                json!({"file_path": file, "content": content}),
            ),
            final_text("Done."),
        ]);
        let mut engine = scoped_stream(&base, &work, &policy, &api, log);
        let mut stdin = engine.0.stdin.take().unwrap();
        stdin.write_all(&user_frame("Write the files")).unwrap();
        drop(stdin);
        let status = exit_within(&mut engine.0, Duration::from_secs(30));
        assert!(
            status.is_some_and(|s| s.success()),
            "the reply did not finish\n{}",
            logs(&base, log)
        );
        let frames = frames(&base, log);
        assert_eq!(
            admission(&frames),
            Some(&receipt(admitted)),
            "the reply was not admitted with the host's current scope\n{}",
            logs(&base, log)
        );
        frames
    };
    // A missing file reads as empty so the failure still prints the run's logs.
    let read = |file: &Path| fs::read_to_string(file).unwrap_or_default();
    let (kept, granted, added, by_tool) = (
        work.join("kept"),
        project.join("granted"),
        project.join("added"),
        project.join("by-tool"),
    );

    // While the project is writable, both the shell and the file tool write there.
    let writable = scope("reply-granted", &work, &[&work, &project], &[]);
    let command = format!("printf one > '{}'", granted.display());
    reply("granted", &writable, &command, &by_tool, "one");
    let log = logs(&base, "granted");
    assert_eq!(read(&granted), "one", "{log}");
    assert_eq!(read(&by_tool), "one", "{log}");

    // The host withdraws write access to the project for the next reply.
    let read_only = scope("reply-downgraded", &work, &[&work], &[&project]);
    let command = format!(
        "printf two > '{}'; printf two > '{}'; printf two > '{}'",
        kept.display(),
        granted.display(),
        added.display()
    );
    let frames = reply("downgraded", &read_only, &command, &by_tool, "two");
    let log = logs(&base, "downgraded");
    assert_eq!(
        read(&kept),
        "two",
        "the shell could not write to the workspace\n{log}"
    );
    assert_eq!(
        read(&granted),
        "one",
        "the shell overwrote a file in the downgraded project\n{log}"
    );
    assert!(
        !added.exists(),
        "the shell created a file in the downgraded project\n{log}"
    );
    assert_eq!(
        read(&by_tool),
        "one",
        "Write overwrote a file in the downgraded project\n{log}"
    );
    let shell = &tool_result(&frames, "shell")["content"][0]["text"];
    for refused in [&granted, &added] {
        let error = format!("{}: Operation not permitted", refused.display());
        assert!(
            shell.as_str().is_some_and(|text| text.contains(&error)),
            "the kernel did not refuse the shell's write to {}\n{shell}\n{log}",
            refused.display()
        );
    }
    // The scope decision refuses the file tool and points back to Chat.
    let write = tool_result(&frames, "write");
    assert_eq!(write["is_error"], true, "{write}\n{log}");
    assert!(
        write["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("Ask for the folder explicitly in Chat")),
        "the scope decision did not refuse Write\n{write}\n{log}"
    );
}

/// The engine side of a downgrade on an open stream. A scoped stream serves
/// one user message (#460), so a message the host sends after narrowing the
/// policy cannot run under the stream's earlier admission. The engine reads
/// the policy only at launch, so the rewrite never reaches the open stream;
/// what keeps the later message out is that the stream ends. The fixture's
/// last two replies are for a stream that kept serving, so they go unused.
#[test]
fn a_scoped_stream_serves_only_its_first_message() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("a-work");
    let project = base.join("b-project");
    for root in [&work, &project] {
        fs::create_dir(root).unwrap();
    }
    let policy = base.join("policy.json");
    let writable = scope("before-downgrade", &work, &[&work, &project], &[]);
    fs::write(&policy, writable.to_string()).unwrap();
    let (ready, go, stale) = (work.join("ready"), work.join("go"), project.join("stale"));
    // The first reply's shell holds the run inside that reply until the test
    // has narrowed the policy and sent the later message (bounded at 30 s).
    let hold = format!(
        "printf ready > '{}'; for _ in $(seq 600); do [ -e '{}' ] && break; sleep 0.05; done",
        ready.display(),
        go.display()
    );
    let (api, requests) = messages_fixture(vec![
        tool_call("hold", "Bash", json!({"command": hold})),
        final_text("First reply."),
        tool_call(
            "stale",
            "Bash",
            json!({"command": format!("printf stale > '{}'", stale.display())}),
        ),
        final_text("Second reply."),
    ]);
    let mut engine = scoped_stream(&base, &work, &policy, &api, "stream");
    let stdin = engine.0.stdin.as_mut().unwrap();
    stdin.write_all(&user_frame("First message")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        let running = engine.0.try_wait().unwrap().is_none();
        assert!(
            running && Instant::now() < deadline,
            "the first reply never started\n{}",
            logs(&base, "stream")
        );
        thread::sleep(Duration::from_millis(10));
    }
    // The run is admitted and inside its first reply. The host narrows the
    // project to read-only, then sends its next message on the same stream,
    // which Coven keeps open.
    let read_only = scope("after-downgrade", &work, &[&work], &[&project]);
    fs::write(&policy, read_only.to_string()).unwrap();
    let stdin = engine.0.stdin.as_mut().unwrap();
    stdin.write_all(&user_frame("A later message")).unwrap();
    fs::write(&go, "").unwrap();
    let status = exit_within(&mut engine.0, Duration::from_secs(30));
    let log = logs(&base, "stream");
    assert!(
        status.is_some_and(|s| s.success()),
        "a scoped stream outlived its first reply\n{log}"
    );
    assert!(
        !stale.exists(),
        "a later message wrote under the earlier admission\n{log}"
    );
    let requests: Vec<String> = requests.try_iter().collect();
    // One request starts the first reply and one returns its tool result.
    assert_eq!(
        requests.len(),
        2,
        "a scoped stream asked the model again: {requests:?}\n{log}"
    );
    assert!(
        requests
            .iter()
            .all(|request| request.starts_with("POST /v1/messages ")),
        "{requests:?}\n{log}"
    );
    let frames = frames(&base, "stream");
    assert_eq!(admission(&frames), Some(&receipt(&writable)), "{log}");
    let results: Vec<&Value> = frames
        .iter()
        .filter(|frame| frame["type"] == "result")
        .collect();
    assert_eq!(
        results.len(),
        1,
        "a scoped stream answered more than one message\n{log}"
    );
    assert_eq!(results[0]["subtype"], "success", "{log}");
}

/// A grant belongs to its reply (`stream_mode.rs`: "any temporary folder
/// approval belongs to this reply only"), and a shell's sandbox is fixed when
/// the shell starts. So a background command started while a project was
/// granted must not keep running after its reply ends, by which time the host
/// may have narrowed the next reply. This pins the engine's side: the
/// background shell it started is killed when the run ends. A process the
/// shell leaves behind (here its `sleep`) is left to the host, which kills the
/// run's whole group once the run exits (`terminate_tree` in coven-cli's
/// pty_runner.rs).
#[test]
fn a_background_command_does_not_outlive_its_reply() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("a-work");
    let project = base.join("b-project");
    for root in [&work, &project] {
        fs::create_dir(root).unwrap();
    }
    let policy = base.join("policy.json");
    let granted = scope("background-reply", &work, &[&work, &project], &[]);
    fs::write(&policy, granted.to_string()).unwrap();
    let (started, late) = (work.join("started"), project.join("late"));
    let delayed = format!(
        "printf %s $$ > '{}'; sleep 10; printf late > '{}'",
        started.display(),
        late.display()
    );
    // A foreground command waits for the background shell, so the reply
    // cannot end before that shell starts (bounded at 30 s).
    let wait = format!(
        "for _ in $(seq 600); do [ -s '{}' ] && break; sleep 0.05; done",
        started.display()
    );
    let (api, _requests) = messages_fixture(vec![
        tool_call(
            "delayed",
            "Bash",
            json!({"command": delayed, "run_in_background": true}),
        ),
        tool_call("wait", "Bash", json!({"command": wait})),
        final_text("Started."),
    ]);
    let mut engine = scoped_stream(&base, &work, &policy, &api, "reply");
    let mut stdin = engine.0.stdin.take().unwrap();
    stdin
        .write_all(&user_frame("Start the delayed write"))
        .unwrap();
    drop(stdin);
    let status = exit_within(&mut engine.0, Duration::from_secs(30));
    let ended_at = Instant::now();
    assert!(
        status.is_some_and(|s| s.success()),
        "the reply did not finish\n{}",
        logs(&base, "reply")
    );
    assert!(
        !late.exists(),
        "the write landed before the reply ended: a stalled runner, or the engine \
         waited for the background command\n{}",
        logs(&base, "reply")
    );
    assert_eq!(
        admission(&frames(&base, "reply")),
        Some(&receipt(&granted)),
        "{}",
        logs(&base, "reply")
    );
    let shell = fs::read_to_string(&started)
        .ok()
        .and_then(|pid| pid.parse().ok())
        .map(Pid::from_raw)
        .unwrap_or_else(|| {
            panic!(
                "the background command never started\n{}",
                logs(&base, "reply")
            )
        });
    if !reaped_within(shell, Duration::from_secs(3)) {
        kill_and_fail(
            shell,
            "the background shell outlived its reply",
            &base,
            "reply",
        );
    }
    // The shell is gone, so nothing should write now. Outlast the command's
    // own delay anyway before looking for its write.
    thread::sleep((ended_at + Duration::from_secs(11)).saturating_duration_since(Instant::now()));
    assert!(
        !late.exists(),
        "a background command wrote after its reply ended\n{}",
        logs(&base, "reply")
    );
}

/// The other reading of a downgrade: a scoped launch never runs with a scope
/// other than the one it was given. If the scope cannot be applied exactly,
/// the launch refuses before any output or model request. One granted project
/// moves away, which the policy check catches. Another cannot be written,
/// which only the kernel probe catches. The same verification (the platform
/// check, then the kernel probe) is what refuses a launch when native
/// enforcement is unavailable (#460).
#[test]
fn a_scoped_launch_refuses_a_scope_it_cannot_apply_exactly() {
    use std::os::unix::fs::PermissionsExt;
    // Root's access checks ignore mode bits, which the second case relies on.
    assert!(!getuid().is_root(), "run this suite as an ordinary user");
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("a-work");
    let (moved, locked) = (base.join("b-moved"), base.join("c-locked"));
    for root in [&work, &moved, &locked] {
        fs::create_dir(root).unwrap();
    }
    let policy = base.join("policy.json");
    let refused = |log: &str, reason: &str| {
        let (api, requests) = messages_fixture(vec![final_text("Unscoped reply.")]);
        let mut engine = scoped_stream(&base, &work, &policy, &api, log);
        if let Some(mut stdin) = engine.0.stdin.take() {
            // The engine may already have refused and closed its end.
            let _ = stdin.write_all(&user_frame("Write the files"));
        }
        let status = exit_within(&mut engine.0, Duration::from_secs(30));
        let output = logs(&base, log);
        assert!(
            status.is_some_and(|s| !s.success()),
            "a scoped launch ran with a scope it could not apply\n{output}"
        );
        let stdout = fs::read_to_string(base.join(format!("{log}.stdout"))).unwrap_or_default();
        assert!(stdout.is_empty(), "a refused launch wrote output\n{output}");
        let stderr = fs::read_to_string(base.join(format!("{log}.stderr"))).unwrap_or_default();
        assert!(stderr.contains(reason), "{output}");
        let requests: Vec<String> = requests.try_iter().collect();
        assert!(
            requests.is_empty(),
            "a refused launch asked the model: {requests:?}\n{output}"
        );
    };

    let policy_for = |run_id: &str, project: &Path| {
        fs::write(
            &policy,
            scope(run_id, &work, &[&work, project], &[]).to_string(),
        )
        .unwrap();
    };
    // What `installed_entrypoint_reports_exact_kernel_scope_and_rejects_a_changed_root`
    // checks, this time through the Chat launch.
    policy_for("moved-root", &moved);
    fs::rename(&moved, base.join("elsewhere")).unwrap();
    refused(
        "moved",
        "Workspace access requires existing canonical dedicated folders.",
    );

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    policy_for("locked-root", &locked);
    refused(
        "locked",
        "The workspace sandbox did not allow exactly the saved folders.",
    );
}
