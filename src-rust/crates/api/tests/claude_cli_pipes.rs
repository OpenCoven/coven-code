#![cfg(unix)]

use claurst_api::{
    provider::LlmProvider,
    provider_types::{ProviderRequest, StopReason},
    providers::ClaudeCliProvider,
};
use claurst_core::types::{ContentBlock, Message};
use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const FAKE: &str = r#"
import json, os, pathlib, sys, time
root = pathlib.Path(os.environ['CLI_FIXTURE_ROOT'])
case = os.environ['CLI_FIXTURE_CASE']
if case == 'resume':
    with (root / 'invocations').open('a') as log:
        log.write(('resume' if '--resume' in sys.argv else 'fresh') + '\n')
    if '--resume' in sys.argv:
        assert sys.argv[sys.argv.index('--resume') + 1] == 'fixture'
        sys.exit(1)
(root / 'pid.tmp').write_text(str(os.getpid()))
os.replace(root / 'pid.tmp', root / 'pid')
def emit(value):
    print(json.dumps(value), flush=True)
if case == 'startup-drop':
    time.sleep(60)
if case == 'stream-drop':
    emit({'type':'system','session_id':'fixture','model':'fixture'})
    time.sleep(60)
if case in ('stdout-first', 'stderr-first'):
    def stdout():
        for _ in range(256):
            emit({'type':'fixture_noise','padding':'x'*8192})
    def stderr():
        sys.stderr.write('e' * (2 * 1024 * 1024))
        sys.stderr.flush()
    if case == 'stdout-first':
        stdout(); stderr()
    else:
        stderr(); stdout()
if case == 'early-close':
    os.close(0)
else:
    value = json.loads(sys.stdin.read())
    expected = 'p' * (2 * 1024 * 1024)
    if case == 'resume' and (root / 'invocations').read_text().splitlines() == ['fresh','resume','fresh']:
        expected = '<conversation-so-far>\nThe session was restarted; this is the conversation before the current message.\nuser: prior\nassistant: reply\n</conversation-so-far>\n\nlatest'
    assert value['message']['content'][0]['text'] == expected
    (root / 'input-verified').write_text('yes')
emit({'type':'system','session_id':'fixture','model':'fixture'})
emit({'type':'assistant','message':{'model':'fixture','content':[{'type':'text','text':'fixture response'}]}})
emit({'type':'result','usage':{'input_tokens':7,'output_tokens':3}})
"#;

struct OwnedCase {
    root: PathBuf,
    child: Option<Child>,
    python: PathBuf,
}
impl OwnedCase {
    fn cleanup(&mut self) -> std::io::Result<()> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        // Never signal the group after reaping its leader.
        let result = Command::new(&self.python).args(["-c", "import os,signal,sys\ntry: os.killpg(int(sys.argv[1]),signal.SIGKILL)\nexcept ProcessLookupError: pass", &child.id().to_string()]).status();
        let group_killed = result.as_ref().is_ok_and(|status| status.success());
        if !group_killed {
            child.kill()?;
        }
        child.wait()?;
        fs::remove_dir_all(&self.root)?;
        if group_killed {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "owned group cleanup failed: {result:?}"
            )))
        }
    }
}
impl Drop for OwnedCase {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("fixture cleanup failed: {error}");
        }
    }
}

fn run_case(case: &str) {
    let python = ["/opt/homebrew/bin/python3", "/usr/bin/python3"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .expect("fixture requires Python 3");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "coven-cli-pipes-{}-{case}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("fake.py"), FAKE).unwrap();
    let launcher = root.join("claude");
    fs::write(
        &launcher,
        "#!/bin/sh\nexec \"$CLI_FIXTURE_PYTHON\" \"$CLI_FIXTURE_ROOT/fake.py\" \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    let log = fs::File::create(root.join("worker.log")).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cli_fixture_worker", "--ignored", "--nocapture"])
        .env_clear()
        .env("HOME", &root)
        .env("USERPROFILE", &root)
        .env("COVEN_CODE_HOME", &root)
        .env("COVEN_HOME", root.join("coven"))
        .env("COVEN_CAVE_HOME", root.join("cave"))
        .env("PATH", &root)
        .env("COVEN_CODE_CLAUDE_BIN", &launcher)
        .env("CLI_FIXTURE_ROOT", &root)
        .env("CLI_FIXTURE_CASE", case)
        .env("CLI_FIXTURE_PYTHON", &python)
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .process_group(0)
        .spawn()
        .unwrap();
    assert!(child.id() >= 2);
    let mut owned = OwnedCase {
        root,
        child: Some(child),
        python,
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while !owned.root.join("complete").exists()
        && !owned.root.join("failed").exists()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    let complete = fs::read_to_string(owned.root.join("complete")).ok();
    let started = owned.root.join("pid").exists();
    let log = fs::read_to_string(owned.root.join("worker.log")).unwrap();
    owned.cleanup().expect("owned process-group cleanup failed");
    assert_eq!(complete.as_deref(), Some("complete"), "{case}: {log}");
    assert!(started, "fake executable was not reached");
}

#[test]
fn stdout_before_large_stdin_completes() {
    run_case("stdout-first");
}
#[test]
fn stderr_before_large_stdin_completes() {
    run_case("stderr-first");
}
#[test]
fn stale_resume_retries_with_full_transcript() {
    run_case("resume");
}
#[test]
fn quiet_child_completes() {
    run_case("quiet");
}
#[test]
fn closed_stdin_preserves_response() {
    run_case("early-close");
}
#[test]
fn dropped_startup_terminates_child() {
    run_case("startup-drop");
}
#[test]
fn dropped_unpolled_stream_terminates_child() {
    run_case("stream-drop");
}

async fn wait_for_pid(root: &Path) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while fs::read_to_string(root.join("pid"))
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

async fn assert_child_gone(root: &Path) {
    let pid = fs::read_to_string(root.join("pid")).unwrap();
    assert!(pid.parse::<u32>().unwrap() >= 2);
    let python = std::env::var_os("CLI_FIXTURE_PYTHON").unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = tokio::process::Command::new(&python).args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]),0)\nexcept ProcessLookupError: sys.exit(0)\nsys.exit(1)", &pid]).status().await.unwrap();
            if status.success() { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("provider child survived drop before group cleanup");
}

#[test]
#[ignore = "invoked by isolated parent drivers"]
fn cli_fixture_worker() {
    let root = PathBuf::from(std::env::var_os("CLI_FIXTURE_ROOT").unwrap());
    let case = std::env::var("CLI_FIXTURE_CASE").unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(async {
        let provider = ClaudeCliProvider::new();
        let mut request = ProviderRequest {
            model: "fixture".into(), messages: vec![Message::user("p".repeat(2 * 1024 * 1024))],
            system_prompt: None, tools: Vec::new(), max_tokens: 16,
            temperature: None, top_p: None, top_k: None, stop_sequences: Vec::new(),
            thinking: None, provider_options: serde_json::json!({}),
        };
        if case == "startup-drop" {
            {
                let startup = provider.create_message_stream(request);
                tokio::pin!(startup);
                tokio::select! {
                    _ = &mut startup => panic!("startup unexpectedly completed"),
                    _ = wait_for_pid(&root) => {}
                }
            }
            assert_child_gone(&root).await;
        } else if case == "stream-drop" {
            let stream = tokio::time::timeout(Duration::from_secs(10), provider.create_message_stream(request)).await.expect("startup stalled").unwrap();
            drop(stream);
            assert_child_gone(&root).await;
        } else {
            if case == "resume" {
                let first = tokio::time::timeout(Duration::from_secs(10), provider.create_message(request.clone())).await.unwrap().unwrap();
                assert!(matches!(first.content.as_slice(), [ContentBlock::Text { text }] if text == "fixture response"));
                request.messages = vec![Message::user("prior"), Message::assistant("reply"), Message::user("latest")];
            }
            let response = tokio::time::timeout(Duration::from_secs(10), provider.create_message(request)).await.expect("provider stalled on pipes").unwrap();
            assert_eq!(response.model, "fixture");
            assert_eq!(response.stop_reason, StopReason::EndTurn);
            assert_eq!(response.usage.input_tokens, 7);
            assert_eq!(response.usage.output_tokens, 3);
            assert!(matches!(response.content.as_slice(), [ContentBlock::Text { text }] if text == "fixture response"));
            if case == "resume" {
                assert_eq!(fs::read_to_string(root.join("invocations")).unwrap(), "fresh\nresume\nfresh\n");
            }
            if case != "early-close" { assert_eq!(fs::read_to_string(root.join("input-verified")).unwrap(), "yes"); }
        }
    })
    }));
    let marker = if outcome.is_ok() {
        "complete"
    } else {
        "failed"
    };
    let pending = root.join("outcome.pending");
    if let Err(error) =
        fs::write(&pending, marker).and_then(|()| fs::rename(&pending, root.join(marker)))
    {
        eprintln!("fixture outcome publication failed: {error}");
    }
    // Keep the group leader owned and unreaped until the parent kills the group.
    std::thread::sleep(Duration::from_secs(60));
}
