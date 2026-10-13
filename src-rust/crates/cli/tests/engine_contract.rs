//! Engine-contract fixtures: the surfaces `coven` drives on a `coven-code`
//! engine, frozen as executable tests.
//!
//! Reference: `OpenCoven/coven` → `docs/ENGINE-CONTRACT.md` (contract v1).
//! Surfaces already covered elsewhere and deliberately not duplicated here:
//!
//! - surface 1 (bare launch exits 0 on quit): `scripts/tui-tests/cases/06_quit.sh`
//! - surface 10 (`acp` JSON-RPC server): `tests/acp_smoke.rs`
//! - headless session-brief / result envelope: `tests/headless_contract/`
//!
//! Every test runs the real binary (`CARGO_BIN_EXE_coven-code`) in a scratch
//! environment: `HOME` and `COVEN_HOME` point inside a temp dir, every
//! credential env var is scrubbed, and the keyless Claude transport is pointed
//! at a fake `claude` script (`COVEN_CODE_CLAUDE_BIN`) that replays a canned
//! stream-json turn. No test touches the developer's real `~/.coven`, and no
//! test makes a network call.
//!
//! Golden files live in `tests/engine_contract/`. To regenerate after a
//! deliberate contract change (which also needs a `contract_version` bump on
//! the coven side):
//!
//! ```text
//! UPDATE_GOLDENS=1 cargo test -p claurst --test engine_contract
//! ```

#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_coven-code");
const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
const RUN_TIMEOUT: Duration = Duration::from_secs(90);

/// Model id passed with `--model`; the fake `claude` echoes it back so the
/// test can prove pass-through.
const FAKE_MODEL: &str = "fake-claude-model";
/// Text the fake `claude` answers with on every turn.
const FAKE_REPLY: &str = "pong";
/// `--session-id` tag used by the stream-json fixtures so `session_id` in the
/// golden frames is deterministic.
const CONTRACT_SESSION: &str = "contract-fixture-session";

/// Env vars that would let real credentials, a real engine home, or a real
/// daemon leak into a fixture run. Scrubbed from every child process.
const SCRUBBED_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_CONFIG_DIR",
    "OPENAI_API_KEY",
    "COVEN_CODE_PROVIDER",
    "COVEN_CODE_API_BASE",
    "COVEN_CODE_HOSTED_REVIEW",
    "COVEN_CODE_HOME",
    "COVEN_CODE_TEST_HOME",
    "COVEN_CODE_CLAUDE_BIN",
    "COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID",
    "COVEN_HOME",
    "COVEN_PARENT",
    "COVEN_DAEMON_SOCKET",
    "COVENCAVE",
    "RUST_LOG",
];

/// Scratch filesystem for one fixture run.
struct Scratch {
    root: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("coven-code-contract-")
            .tempdir()
            .expect("create scratch dir");
        let scratch = Self { root };
        for dir in [
            scratch.home(),
            scratch.coven_home(),
            scratch.cwd(),
            scratch.bin_dir(),
        ] {
            fs::create_dir_all(&dir).expect("create scratch subdir");
        }
        scratch.write_fake_claude();
        scratch
    }

    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }
    fn coven_home(&self) -> PathBuf {
        self.root.path().join("coven")
    }
    fn cwd(&self) -> PathBuf {
        self.root.path().join("cwd")
    }
    fn bin_dir(&self) -> PathBuf {
        self.root.path().join("bin")
    }
    fn claude_bin(&self) -> PathBuf {
        self.bin_dir().join("claude")
    }
    /// argv the fake `claude` received on its last invocation, one per line.
    fn claude_argv_log(&self) -> PathBuf {
        self.root.path().join("claude-argv.log")
    }
    /// stdin the fake `claude` received on its last invocation.
    fn claude_stdin_log(&self) -> PathBuf {
        self.root.path().join("claude-stdin.log")
    }

    /// A stand-in for the `claude` CLI: records argv and stdin, then replays
    /// one canned stream-json turn. This is the engine's keyless Claude
    /// transport (`crates/api/src/providers/claude_cli.rs`), so `--print` and
    /// stream-json runs exercise the real query loop with no network.
    fn write_fake_claude(&self) {
        let script = format!(
            "#!/bin/sh\n\
             # Fake `claude` for coven-code engine-contract fixtures.\n\
             printf '%s\\n' \"$@\" > {argv}\n\
             cat > {stdin}\n\
             printf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"fake-claude-session\",\"model\":\"{model}\"}}'\n\
             printf '%s\\n' '{{\"type\":\"assistant\",\"message\":{{\"model\":\"{model}\",\"content\":[{{\"type\":\"text\",\"text\":\"{reply}\"}}]}}}}'\n\
             printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"usage\":{{\"input_tokens\":3,\"output_tokens\":1}}}}'\n",
            argv = shell_quote(&self.claude_argv_log()),
            stdin = shell_quote(&self.claude_stdin_log()),
            model = FAKE_MODEL,
            reply = FAKE_REPLY,
        );
        let path = self.claude_bin();
        fs::write(&path, script).expect("write fake claude");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fake claude");
    }

    /// A `coven-code` command with the scrubbed, scratch-rooted environment
    /// coven would hand the engine: `HOME` and `COVEN_HOME` inside the
    /// scratch dir, no credentials, fake `claude` on `COVEN_CODE_CLAUDE_BIN`.
    fn engine(&self) -> Command {
        let mut cmd = Command::new(BIN);
        for var in SCRUBBED_ENV {
            cmd.env_remove(var);
        }
        cmd.env("HOME", self.home())
            .env("USER", "contract-fixture-user")
            .env("COVEN_HOME", self.coven_home())
            .env("COVEN_CODE_CLAUDE_BIN", self.claude_bin())
            .current_dir(self.cwd());
        cmd
    }

    fn claude_argv(&self) -> Vec<String> {
        fs::read_to_string(self.claude_argv_log())
            .expect("fake claude recorded argv")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn claude_stdin(&self) -> String {
        fs::read_to_string(self.claude_stdin_log()).expect("fake claude recorded stdin")
    }
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn stdout_frames(&self) -> Vec<serde_json::Value> {
        self.stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("non-JSON line on stdout: {e}\n{line}\n{self}"))
            })
            .collect()
    }
}

impl std::fmt::Display for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "--- exit code: {}\n--- stdout:\n{}\n--- stderr:\n{}",
            self.code, self.stdout, self.stderr
        )
    }
}

/// Spawn `cmd`, feed `stdin` (then close it so EOF is delivered), and wait
/// at most `RUN_TIMEOUT`.
fn run_engine(cmd: &mut Command, stdin: &str) -> Run {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn coven-code");
    {
        let mut handle = child.stdin.take().expect("stdin");
        handle.write_all(stdin.as_bytes()).expect("write stdin");
    }
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("coven-code did not exit within {RUN_TIMEOUT:?}");
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    let output = child.wait_with_output().expect("wait_with_output");
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/engine_contract")
        .join(name)
}

/// Compare `actual` to the checked-in golden, or rewrite the golden when
/// `UPDATE_GOLDENS` is set.
fn assert_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}\nregenerate with: UPDATE_GOLDENS=1 cargo test -p claurst --test engine_contract",
            path.display()
        )
    });
    if expected != actual {
        let mut report = String::new();
        for (n, (want, got)) in expected.lines().zip(actual.lines()).enumerate() {
            if want != got {
                report.push_str(&format!("line {}:\n  want: {want}\n  got:  {got}\n", n + 1));
            }
        }
        panic!(
            "golden mismatch for {} (expected {} lines, got {}). \
             If the contract changed on purpose, bump contract_version in coven and \
             regenerate with UPDATE_GOLDENS=1.\n{report}--- actual ---\n{actual}",
            path.display(),
            expected.lines().count(),
            actual.lines().count(),
        );
    }
}

fn user_frame(text: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": text },
        })
    )
}

fn stream_json_args() -> [&'static str; 5] {
    [
        "--print",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
    ]
}

/// Strip the per-run values (`cwd`, `duration_ms`) so frames compare
/// byte-for-byte against the golden.
fn normalize_frame(frame: &serde_json::Value) -> serde_json::Value {
    let mut frame = frame.clone();
    if let Some(obj) = frame.as_object_mut() {
        if obj.contains_key("cwd") {
            obj.insert("cwd".to_string(), serde_json::json!("<CWD>"));
        }
        if obj.contains_key("duration_ms") {
            obj.insert("duration_ms".to_string(), serde_json::json!(0));
        }
    }
    frame
}

/// Recursively collect every regular file under `dir`.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn state_mentions_session(root: &Path) -> bool {
    files_under(root).iter().any(|path| {
        fs::read_to_string(path).is_ok_and(|content| content.contains(CONTRACT_SESSION))
    })
}

// ---------------------------------------------------------------------------
// Surface 2: --version
// ---------------------------------------------------------------------------

#[test]
fn contract_version_is_single_line_semver() {
    let scratch = Scratch::new();
    let run = run_engine(scratch.engine().arg("--version"), "");
    assert_eq!(run.code, 0, "{run}");
    assert_eq!(
        run.stdout,
        format!("coven-code {ENGINE_VERSION}\n"),
        "--version must print exactly `coven-code <semver>` on one line\n{run}"
    );
    let semver_ok = ENGINE_VERSION.split('.').count() == 3
        && ENGINE_VERSION
            .split('.')
            .all(|part| part.parse::<u64>().is_ok());
    assert!(
        semver_ok,
        "workspace version is not x.y.z: {ENGINE_VERSION}"
    );
}

// ---------------------------------------------------------------------------
// Surfaces 3–8, 11: the flags coven passes must stay accepted
// ---------------------------------------------------------------------------

#[test]
fn contract_flags_coven_invokes_are_accepted() {
    let scratch = Scratch::new();
    let run = run_engine(scratch.engine().arg("--help"), "");
    assert_eq!(run.code, 0, "{run}");
    for flag in [
        "--print",
        "--input-format",
        "--output-format",
        "--resume",
        "--session-id",
        "--model",
        "--append-system-prompt",
        "--cwd",
        "--permission-mode",
        "--effort",
    ] {
        assert!(
            run.stdout.contains(flag),
            "--help no longer lists contract flag {flag}\n{run}"
        );
    }

    // An unknown flag is a usage error. The contract reserves every exit code
    // other than 0/1; clap's usage-error code (2) is the frozen observed value.
    let run = run_engine(scratch.engine().arg("--definitely-not-a-contract-flag"), "");
    assert_eq!(run.code, 2, "unknown flag must be a usage error\n{run}");
}

#[test]
fn contract_cwd_flag_is_honored() {
    let scratch = Scratch::new();
    let project = scratch.root.path().join("project-via-cwd-flag");
    fs::create_dir_all(&project).expect("create project dir");
    // `--dump-system-prompt` is the offline path that echoes the resolved
    // working directory; it never reaches a provider.
    let run = run_engine(
        scratch
            .engine()
            .args(["--dump-system-prompt", "--cwd"])
            .arg(&project),
        "",
    );
    assert_eq!(run.code, 0, "{run}");
    assert!(
        run.stdout.contains(&project.display().to_string()),
        "--cwd was not honored in the system context\n{run}"
    );
}

#[test]
fn contract_permission_mode_values_are_accepted() {
    let scratch = Scratch::new();
    for mode in ["default", "accept-edits", "bypass-permissions", "plan"] {
        let run = run_engine(
            scratch
                .engine()
                .args(["--permission-mode", mode, "--dump-system-prompt"]),
            "",
        );
        assert_eq!(run.code, 0, "--permission-mode {mode} rejected\n{run}");
    }
    let run = run_engine(
        scratch
            .engine()
            .args(["--permission-mode", "not-a-mode", "--dump-system-prompt"]),
        "",
    );
    assert_eq!(
        run.code, 2,
        "invalid --permission-mode must be a usage error\n{run}"
    );
}

#[test]
fn contract_effort_levels_are_accepted() {
    let scratch = Scratch::new();
    for level in ["low", "medium", "high", "max"] {
        let run = run_engine(
            scratch
                .engine()
                .args(["--effort", level, "--dump-system-prompt"]),
            "",
        );
        assert_eq!(run.code, 0, "--effort {level} rejected\n{run}");
    }
}

// ---------------------------------------------------------------------------
// Surface 3: --print <prompt>
// ---------------------------------------------------------------------------

#[test]
fn contract_print_writes_result_to_stdout_and_exits_zero() {
    let scratch = Scratch::new();
    let run = run_engine(
        scratch
            .engine()
            .args(["--print", "ping", "--model", FAKE_MODEL]),
        "",
    );
    assert_eq!(run.code, 0, "{run}");
    assert_eq!(
        run.stdout,
        format!("{FAKE_REPLY}\n"),
        "--print must write the result text to stdout\n{run}"
    );

    // The engine drove the keyless Claude transport with the documented
    // argv shape and passed `--model` through untouched.
    let argv = scratch.claude_argv();
    assert!(
        argv.windows(5).any(|w| w
            == [
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json"
            ]),
        "claude CLI argv shape changed: {argv:?}"
    );
    assert!(
        argv.windows(2).any(|w| w == ["--model", FAKE_MODEL]),
        "--model was not passed through to the provider: {argv:?}"
    );
    assert!(
        scratch.claude_stdin().contains("ping"),
        "prompt did not reach the provider:\n{}",
        scratch.claude_stdin()
    );
}

// ---------------------------------------------------------------------------
// Exit codes (headless): 0 = completed, 1 = errored
// ---------------------------------------------------------------------------

#[test]
fn contract_print_without_prompt_exits_one() {
    let scratch = Scratch::new();
    // No positional prompt and an empty stdin: nothing to run.
    let run = run_engine(scratch.engine().arg("--print"), "");
    assert_eq!(run.code, 1, "{run}");
    assert!(
        run.stderr.contains("No prompt provided"),
        "expected the no-prompt error on stderr\n{run}"
    );
}

#[test]
fn contract_print_provider_failure_exits_one() {
    let scratch = Scratch::new();
    let missing = scratch.bin_dir().join("claude-that-does-not-exist");
    let run = run_engine(
        scratch
            .engine()
            .env("COVEN_CODE_CLAUDE_BIN", &missing)
            .args(["--print", "ping"]),
        "",
    );
    assert_eq!(
        run.code, 1,
        "a provider failure must exit 1 (errored)\n{run}"
    );
    assert!(
        run.stderr.contains("claude"),
        "stderr should name the failed transport\n{run}"
    );
}

// ---------------------------------------------------------------------------
// Surface 4: --print --input-format stream-json --output-format stream-json
// ---------------------------------------------------------------------------

#[test]
fn contract_stream_json_turn_matches_golden_frames() {
    let scratch = Scratch::new();
    let mut stdin = String::new();
    // Unknown `type` values are silently ignored.
    stdin.push_str("{\"type\":\"control_request\",\"request\":{}}\n");
    // Legacy-shape assistant frames append prefill without running a turn.
    stdin.push_str("{\"role\":\"assistant\",\"content\":\"prefill\"}\n");
    // Primary shape: triggers the turn.
    stdin.push_str(&user_frame("ping"));

    let run = run_engine(
        scratch.engine().args(stream_json_args()).args([
            "--session-id",
            CONTRACT_SESSION,
            "--model",
            FAKE_MODEL,
        ]),
        &stdin,
    );
    assert_eq!(run.code, 0, "stream loop must exit 0 on stdin EOF\n{run}");

    let frames = run.stdout_frames();
    let kinds: Vec<&str> = frames
        .iter()
        .map(|f| f["type"].as_str().unwrap_or("<missing type>"))
        .collect();
    assert_eq!(
        kinds,
        ["system", "assistant", "result"],
        "one text-only turn must emit exactly system(init), assistant, result\n{run}"
    );

    // Per-event-type fields coven parses (ENGINE-CONTRACT.md "Stream-json events").
    let init = &frames[0];
    assert_eq!(init["subtype"], "init", "{run}");
    assert!(init["cwd"].is_string(), "system.init.cwd\n{run}");
    assert_eq!(init["session_id"], CONTRACT_SESSION, "{run}");
    assert!(init["tools"].is_array(), "system.init.tools\n{run}");
    assert_eq!(init["model"], FAKE_MODEL, "system.init.model\n{run}");

    let assistant = &frames[1];
    assert_eq!(assistant["message"]["role"], "assistant", "{run}");
    assert_eq!(
        assistant["message"]["content"][0],
        serde_json::json!({ "type": "text", "text": FAKE_REPLY }),
        "{run}"
    );
    assert_eq!(assistant["session_id"], CONTRACT_SESSION, "{run}");
    assert_eq!(assistant["stop_reason"], "end_turn", "{run}");

    let result = &frames[2];
    assert_eq!(result["subtype"], "success", "{run}");
    assert!(result["duration_ms"].is_u64(), "result.duration_ms\n{run}");
    assert_eq!(result["is_error"], false, "{run}");
    assert!(result["num_turns"].is_u64(), "result.num_turns\n{run}");
    assert_eq!(result["session_id"], CONTRACT_SESSION, "{run}");
    assert!(result["error"].is_null(), "result.error on success\n{run}");

    // The prefill frame reached the provider as conversation context, the
    // ignored frame did not.
    let provider_stdin = scratch.claude_stdin();
    assert!(provider_stdin.contains("prefill"), "{provider_stdin}");
    assert!(
        !provider_stdin.contains("control_request"),
        "{provider_stdin}"
    );

    let golden: String = frames
        .iter()
        .map(|f| format!("{}\n", normalize_frame(f)))
        .collect();
    assert_golden("stream_json_turn.golden.jsonl", &golden);
}

#[test]
fn contract_stream_json_exits_zero_on_eof_without_a_turn() {
    let scratch = Scratch::new();
    let run = run_engine(
        scratch
            .engine()
            .args(stream_json_args())
            .args(["--session-id", CONTRACT_SESSION]),
        "",
    );
    assert_eq!(run.code, 0, "{run}");
    let frames = run.stdout_frames();
    assert_eq!(frames.len(), 1, "only system.init before EOF\n{run}");
    assert_eq!(frames[0]["type"], "system", "{run}");
    assert_eq!(frames[0]["subtype"], "init", "{run}");
    assert_eq!(frames[0]["session_id"], CONTRACT_SESSION, "{run}");
    // The fake provider was never spawned.
    assert!(
        !scratch.claude_argv_log().exists(),
        "no turn must run on EOF"
    );
}

// ---------------------------------------------------------------------------
// Surfaces 5 + 6: --session-id tags a run; --resume restores it
// ---------------------------------------------------------------------------

#[test]
fn contract_resume_replays_a_session_tagged_by_session_id() {
    let scratch = Scratch::new();

    // Turn 1 under an explicit session id.
    let first = run_engine(
        scratch
            .engine()
            .args(stream_json_args())
            .args(["--session-id", CONTRACT_SESSION]),
        &user_frame("ping"),
    );
    assert_eq!(first.code, 0, "{first}");
    assert!(
        state_mentions_session(&scratch.coven_home()),
        "turn 1 must persist the session under COVEN_HOME"
    );

    // Turn 2 in a fresh process resumes it: the init frame keeps the id and
    // the provider receives the earlier exchange as context.
    let second = run_engine(
        scratch
            .engine()
            .args(stream_json_args())
            .args(["--resume", CONTRACT_SESSION]),
        &user_frame("second question"),
    );
    assert_eq!(second.code, 0, "{second}");
    let frames = second.stdout_frames();
    assert_eq!(frames[0]["type"], "system", "{second}");
    assert_eq!(
        frames[0]["session_id"], CONTRACT_SESSION,
        "--resume must keep the session id\n{second}"
    );
    let provider_stdin = scratch.claude_stdin();
    for needle in ["ping", FAKE_REPLY, "second question"] {
        assert!(
            provider_stdin.contains(needle),
            "resumed turn is missing `{needle}` from the transcript:\n{provider_stdin}"
        );
    }
}

// ---------------------------------------------------------------------------
// Surface 9: auth status --json (offline)
// ---------------------------------------------------------------------------

#[test]
fn contract_auth_status_json_reports_logged_out_offline() {
    let scratch = Scratch::new();
    let run = run_engine(scratch.engine().args(["auth", "status", "--json"]), "");
    assert_eq!(run.code, 1, "loggedIn:false must exit 1\n{run}");
    let status: serde_json::Value = serde_json::from_str(&run.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\n{run}"));
    assert_eq!(status["loggedIn"], false, "{run}");
}

#[test]
fn contract_auth_status_json_reports_logged_in_from_env_key() {
    let scratch = Scratch::new();
    let run = run_engine(
        scratch
            .engine()
            .env("ANTHROPIC_API_KEY", "contract-fixture-key")
            .args(["auth", "status", "--json"]),
        "",
    );
    assert_eq!(run.code, 0, "loggedIn:true must exit 0\n{run}");
    let status: serde_json::Value = serde_json::from_str(&run.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\n{run}"));
    assert_eq!(status["loggedIn"], true, "{run}");
}

// ---------------------------------------------------------------------------
// Environment: COVEN_HOME / COVEN_PARENT / COVEN_DAEMON_SOCKET
// ---------------------------------------------------------------------------

#[test]
fn contract_coven_home_scopes_engine_state() {
    let scratch = Scratch::new();
    let run = run_engine(
        scratch
            .engine()
            .args(stream_json_args())
            .args(["--session-id", CONTRACT_SESSION]),
        &user_frame("ping"),
    );
    assert_eq!(run.code, 0, "{run}");
    assert!(
        state_mentions_session(&scratch.coven_home().join("code")),
        "with COVEN_HOME set, engine state must live under $COVEN_HOME/code"
    );
    assert!(
        !scratch.home().join(".coven-code").exists(),
        "legacy ~/.coven-code must not be created when COVEN_HOME is set"
    );
}

#[test]
fn contract_coven_parent_scopes_engine_state_under_home() {
    let scratch = Scratch::new();
    let run = run_engine(
        scratch
            .engine()
            .env_remove("COVEN_HOME")
            .env("COVEN_PARENT", "coven")
            .args(stream_json_args())
            .args(["--session-id", CONTRACT_SESSION]),
        &user_frame("ping"),
    );
    assert_eq!(run.code, 0, "{run}");
    assert!(
        state_mentions_session(&scratch.home().join(".coven").join("code")),
        "with COVEN_PARENT=coven and no COVEN_HOME, state must live under ~/.coven/code"
    );
    assert!(
        !scratch.home().join(".coven-code").exists(),
        "legacy ~/.coven-code must not be created under COVEN_PARENT=coven"
    );
}

#[test]
fn contract_standalone_state_lives_in_legacy_home() {
    let scratch = Scratch::new();
    let run = run_engine(
        scratch
            .engine()
            .env_remove("COVEN_HOME")
            .args(stream_json_args())
            .args(["--session-id", CONTRACT_SESSION]),
        &user_frame("ping"),
    );
    assert_eq!(run.code, 0, "{run}");
    assert!(
        state_mentions_session(&scratch.home().join(".coven-code")),
        "standalone runs must keep state in ~/.coven-code"
    );
    assert!(
        !scratch.coven_home().join("code").exists(),
        "an unset COVEN_HOME must not create $COVEN_HOME/code"
    );
}

#[test]
fn contract_coven_daemon_socket_is_inherited_without_side_effects() {
    // The contract reserves COVEN_DAEMON_SOCKET for the daemon-session
    // notifier and only promises it is inherited. The engine must neither
    // fail nor change its output when the variable points nowhere.
    let scratch = Scratch::new();
    let bogus = scratch.root.path().join("no-such-daemon.sock");
    let version = run_engine(
        scratch
            .engine()
            .env("COVEN_DAEMON_SOCKET", &bogus)
            .arg("--version"),
        "",
    );
    assert_eq!(version.code, 0, "{version}");
    assert_eq!(version.stdout, format!("coven-code {ENGINE_VERSION}\n"));

    let stream = run_engine(
        scratch
            .engine()
            .env("COVEN_DAEMON_SOCKET", &bogus)
            .args(stream_json_args())
            .args(["--session-id", CONTRACT_SESSION]),
        &user_frame("ping"),
    );
    assert_eq!(stream.code, 0, "{stream}");
    let frames = stream.stdout_frames();
    let kinds: Vec<&str> = frames
        .iter()
        .map(|f| f["type"].as_str().unwrap_or("<missing type>"))
        .collect();
    assert_eq!(kinds, ["system", "assistant", "result"], "{stream}");
}
