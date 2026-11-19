//! #535: coven-code exits once its run is complete, while a process that a
//! background shell command started is still running and still holds the
//! command's output pipes. On Windows the runtime reads a child's pipes on
//! its blocking pool, and returning from `main` waited for those reads, which
//! end only when every process holding the pipe has exited.
//!
//! The process keeps running after coven-code exits: a background command
//! outlives the turn and coven-code until it is stopped (#521, #554).

use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

/// How long a run may take, from start to exit.
const RUN_LIMIT: Duration = Duration::from_secs(60);
/// How long the holder runs if nothing stops it.
const HOLDER_LIFETIME: Duration = Duration::from_secs(180);
/// The holder runs only in a folder that holds this file.
const MARKER: &str = "holder-535";
const FINAL_TEXT: &str = "The holder is running.";

/// The text of the `--print` run.
#[test]
fn print_run_exits_while_a_background_descendant_holds_its_pipes() {
    let run = Run::start(false);
    let (status, stdout, stderr) = run.finish();
    assert!(status.success(), "{status:?}\n{stdout}\n{stderr}");
    assert_eq!(stdout.trim(), FINAL_TEXT, "{stderr}");
}

/// The stream-json loop ends at the end of its input, then exits the same way.
#[test]
fn stream_json_run_exits_while_a_background_descendant_holds_its_pipes() {
    let run = Run::start(true);
    let (status, stdout, stderr) = run.finish();
    assert!(status.success(), "{status:?}\n{stdout}\n{stderr}");
    let result = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|frame| frame["type"] == "result")
        .unwrap_or_else(|| panic!("no result frame:\n{stdout}\n{stderr}"));
    assert_eq!(result["subtype"], "success", "{result}");
    assert!(stdout.contains(FINAL_TEXT), "{stdout}");
}

/// One engine run whose model starts the holder as a background command,
/// then answers with [`FINAL_TEXT`] once the holder runs.
struct Run {
    directory: tempfile::TempDir,
    engine: Option<Child>,
    started: Instant,
}

impl Run {
    fn start(stream: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().to_path_buf();
        let holder = base.join("holder");
        fs::create_dir(&holder).unwrap();
        fs::write(holder.join(MARKER), "").unwrap();
        let work = base.join("work");
        fs::create_dir(&work).unwrap();
        let api = serve(holder_command(&holder), holder.clone());
        // Output goes to files: the holder inherits no handle of the test's.
        let capture = |stream: &str| fs::File::create(base.join(stream)).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_coven-code"));
        command
            .env_clear()
            // Windows cannot start a networked process without SystemRoot.
            .envs(
                ["PATH", "SystemRoot", "TEMP", "TMP"]
                    .into_iter()
                    .filter_map(|key| std::env::var_os(key).map(|value| (key, value))),
            )
            .env("HOME", &base)
            .env("COVEN_HOME", base.join("coven"))
            .env("COVEN_CAVE_HOME", base.join("cave"))
            .env("COVEN_CODE_HOME", base.join("code"))
            .env("COVEN_CODE_DISABLE_MODELS_FETCH", "1")
            .env("ANTHROPIC_API_KEY", "local-fixture")
            .env("ANTHROPIC_BASE_URL", &api)
            .current_dir(&work)
            .args([
                "--print",
                "--provider",
                "anthropic",
                "--model",
                "claude-fixture",
                "--permission-mode",
                "bypass-permissions",
            ])
            .stdout(capture("stdout"))
            .stderr(capture("stderr"));
        if stream {
            command
                .args([
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                ])
                .stdin(Stdio::piped());
        } else {
            command.arg("Start the holder.").stdin(Stdio::null());
        }
        let started = Instant::now();
        let mut engine = command.spawn().unwrap();
        if let Some(mut stdin) = engine.stdin.take() {
            let frame = json!({"type": "user", "message": {"role": "user", "content": "Start the holder."}});
            writeln!(stdin, "{frame}").unwrap();
        }
        Self {
            directory,
            engine: Some(engine),
            started,
        }
    }

    fn holder(&self) -> PathBuf {
        self.directory.path().join("holder")
    }

    /// The holder's last count. A read can fail while the holder replaces
    /// the file on Windows, so a few are tried.
    fn ticks(&self) -> Option<u64> {
        (0..10).find_map(|_| {
            let ticks = fs::read_to_string(self.holder().join("ticks"))
                .ok()
                .and_then(|text| text.parse().ok());
            if ticks.is_none() {
                thread::sleep(Duration::from_millis(20));
            }
            ticks
        })
    }

    /// Wait for the engine to exit, then check that the holder still runs
    /// and stop it. Returns the engine's status, stdout and stderr.
    fn finish(mut self) -> (ExitStatus, String, String) {
        let mut engine = self.engine.take().unwrap();
        let deadline = self.started + RUN_LIMIT;
        let exited = loop {
            if let Some(status) = engine.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                break None;
            }
            thread::sleep(Duration::from_millis(50));
        };
        let read = |stream: &str| {
            fs::read_to_string(self.directory.path().join(stream)).unwrap_or_default()
        };
        let Some(status) = exited else {
            // Stop the holder, and see whether that lets the engine exit.
            let running = self.ticks().is_some();
            fs::write(self.holder().join("stop"), "").unwrap();
            let stopped = Instant::now();
            let after = loop {
                if let Some(status) = engine.try_wait().unwrap() {
                    break format!(
                        "it exited ({status}) {:?} after the holder was stopped",
                        stopped.elapsed()
                    );
                }
                if stopped.elapsed() >= Duration::from_secs(30) {
                    let _ = engine.kill();
                    let _ = engine.wait();
                    break "it was still running 30 s after the holder was stopped".to_string();
                }
                thread::sleep(Duration::from_millis(50));
            };
            panic!(
                "coven-code did not exit within {RUN_LIMIT:?} (holder running: {running}); {after}\n--- stdout\n{}\n--- stderr\n{}",
                read("stdout"),
                read("stderr"),
            );
        };
        // The holder outlives the engine, as a background command does.
        let before = self.ticks();
        thread::sleep(Duration::from_secs(1));
        let after = self.ticks();
        assert!(
            before.is_some() && after > before && !self.holder().join("stopped").exists(),
            "the background command's descendant stopped when coven-code exited: {before:?} ticks, then {after:?}"
        );
        (status, read("stdout"), read("stderr"))
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = fs::write(self.holder().join("stop"), "");
        if let Some(engine) = &mut self.engine {
            let _ = engine.kill();
            let _ = engine.wait();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.ticks().is_some()
            && !self.holder().join("stopped").exists()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// The background command: this test binary as [`holder`], with stdout sent
/// to the null device. Its stderr is the command's pipe, and it writes
/// nothing there, so a reader of the pipe waits until it exits.
fn holder_command(folder: &Path) -> String {
    let null = if cfg!(windows) { "nul" } else { "/dev/null" };
    // `; true` keeps bash from replacing itself with the holder, so on every
    // platform the holder is a descendant of the shell, not the shell.
    let tail = if cfg!(windows) { "" } else { "; true" };
    format!(
        r#""{}" --exact holder --ignored --nocapture "{}" >{null}{tail}"#,
        std::env::current_exe().unwrap().display(),
        folder.display(),
    )
}

/// A loopback stand-in for the Messages API. A request without a tool result
/// is answered with a Bash call that starts `command` in the background.
/// Otherwise the answer is [`FINAL_TEXT`], once the holder in `holder` runs.
/// The thread stays blocked in `accept` until the test binary exits.
fn serve(command: String, holder: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(socket) = socket else { return };
            answer(socket, &command, &holder);
        }
    });
    base
}

fn answer(socket: TcpStream, command: &str, holder: &Path) {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(socket);
    let mut length = 0;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if line == "\r\n" => break,
            Ok(_) => {}
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or_default();
    let answered = request["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .and_then(|message| message["content"].as_array())
        .is_some_and(|blocks| {
            blocks
                .iter()
                .any(|block| block["type"] == "tool_result" && block["tool_use_id"] == "holder")
        });
    let (block, delta, stop_reason) = if answered {
        // The run must not end before the holder has started.
        let deadline = Instant::now() + Duration::from_secs(30);
        while !holder.join("ticks").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        (
            json!({"type": "text", "text": ""}),
            json!({"type": "text_delta", "text": FINAL_TEXT}),
            "end_turn",
        )
    } else {
        let input = json!({"command": command, "run_in_background": true});
        (
            json!({"type": "tool_use", "id": "holder", "name": "Bash", "input": {}}),
            json!({"type": "input_json_delta", "partial_json": input.to_string()}),
            "tool_use",
        )
    };
    let reply: String = [
        json!({"type": "message_start", "message": {
            "id": "fixture", "type": "message", "role": "assistant", "content": [],
            "model": "claude-fixture", "usage": {"input_tokens": 1, "output_tokens": 0},
        }}),
        json!({"type": "content_block_start", "index": 0, "content_block": block}),
        json!({"type": "content_block_delta", "index": 0, "delta": delta}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": stop_reason},
            "usage": {"output_tokens": 1}}),
        json!({"type": "message_stop"}),
    ]
    .iter()
    .map(|event| {
        format!(
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap()
        )
    })
    .collect();
    let _ = write!(
        reader.into_inner(),
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
        reply.len()
    );
}

/// Started by the background command of the tests above, with its folder as
/// the last argument. It writes a rising count to `ticks` until `stop`
/// appears or [`HOLDER_LIFETIME`] passes, then writes `stopped`. It does
/// nothing in a folder without [`MARKER`].
#[test]
#[ignore = "started by the background command of the tests above"]
fn holder() {
    let Some(folder) = std::env::args_os().next_back().map(PathBuf::from) else {
        return;
    };
    if !folder.join(MARKER).exists() {
        return;
    }
    // Leave the run's folders, so that they can be removed while this runs.
    let _ = std::env::set_current_dir(std::env::temp_dir());
    let started = Instant::now();
    let mut ticks = 0u64;
    while started.elapsed() < HOLDER_LIFETIME && !folder.join("stop").exists() {
        ticks += 1;
        // A reader may hold `ticks` open on Windows; the next tick retries.
        let partial = folder.join("ticks.partial");
        if fs::write(&partial, ticks.to_string()).is_ok() {
            let _ = fs::rename(&partial, folder.join("ticks"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    fs::write(folder.join("stopped"), "").unwrap();
}
