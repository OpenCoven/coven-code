//! A scoped launch on a platform where Coven Code has no workspace sandbox.
#![cfg(not(target_os = "macos"))]

use std::{
    ffi::OsStr,
    fs,
    io::Write,
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

mod workspace_policy_fixture;
use workspace_policy_fixture::{final_text, messages_fixture, scope, user_frame};

/// Only macOS has a native workspace sandbox (#460). On any other platform a
/// valid policy is still refused: the kernel check entrypoint and a scoped
/// Chat stream both exit non-zero, write nothing on stdout and never ask the
/// model.
/// The policy has the same shape as the ones the macOS tests admit, and the
/// refusal asserted is the platform's, so the policy is not what is refused.
#[test]
fn a_scoped_launch_without_a_sandbox_backend_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("a-work");
    fs::create_dir(&work).unwrap();
    let policy = base.join("policy.json");
    fs::write(
        &policy,
        scope("no-backend", &work, &[&work], &[]).to_string(),
    )
    .unwrap();
    let (api, requests) = messages_fixture(vec![final_text("Unscoped reply.")]);
    // Output goes to files, so a run that writes a lot cannot fill a pipe and
    // stall until the time limit.
    let run = |log: &str, args: &[&OsStr]| -> (ExitStatus, String, String) {
        let capture =
            |stream: &str| fs::File::create(base.join(format!("{log}.{stream}"))).unwrap();
        let mut engine = Command::new(env!("CARGO_BIN_EXE_coven-code"))
            .env_clear()
            // Windows cannot start a networked process without SystemRoot.
            .envs(
                ["PATH", "SystemRoot"]
                    .into_iter()
                    .filter_map(|key| std::env::var_os(key).map(|value| (key, value))),
            )
            .env("HOME", &base)
            .env("COVEN_HOME", base.join("coven"))
            .env("COVEN_CAVE_HOME", base.join("cave"))
            .env("COVEN_CODE_HOME", base.join("code"))
            .env("ANTHROPIC_API_KEY", "local-fixture")
            .env("ANTHROPIC_BASE_URL", &api)
            // The engine canonicalizes its working folder itself. On Windows
            // the canonical form is a verbatim path, which is not a safe
            // working directory for a new process.
            .current_dir(temp.path().join("a-work"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(capture("stdout"))
            .stderr(capture("stderr"))
            .spawn()
            .unwrap();
        if let Some(mut stdin) = engine.stdin.take() {
            // The engine may already have refused and closed its end.
            let _ = stdin.write_all(&user_frame("Write the files"));
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = engine.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = engine.kill();
                break engine.wait().unwrap();
            }
            thread::sleep(Duration::from_millis(20));
        };
        let read = |stream: &str| {
            fs::read_to_string(base.join(format!("{log}.{stream}"))).unwrap_or_default()
        };
        (status, read("stdout"), read("stderr"))
    };
    let check = [
        OsStr::new("--verify-workspace-policy-v1"),
        policy.as_os_str(),
    ];
    let stream = [
        "--print",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--provider",
        "anthropic",
        "--chat-workspace-policy-v1",
    ]
    .map(OsStr::new);
    let stream: Vec<&OsStr> = stream.into_iter().chain([policy.as_os_str()]).collect();
    for (launch, log, args) in [
        ("the policy check entrypoint", "check", &check[..]),
        ("a scoped stream", "stream", &stream[..]),
    ] {
        let (status, stdout, stderr) = run(log, args);
        let output = format!("{status:?}\n--- stdout\n{stdout}\n--- stderr\n{stderr}");
        assert!(
            !status.success(),
            "{launch} ran without a sandbox backend: {output}"
        );
        assert!(stdout.is_empty(), "{launch} wrote output: {output}");
        assert!(
            stderr.contains(
                "This Coven Code build cannot establish a workspace sandbox on this platform."
            ),
            "{launch} was not refused for want of a sandbox: {output}"
        );
    }
    let requests: Vec<String> = requests.try_iter().collect();
    assert!(
        requests.is_empty(),
        "a refused launch asked the model: {requests:?}"
    );
}
