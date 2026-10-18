#![cfg(unix)]
use claurst_api::{
    provider::LlmProvider, provider_error::ProviderError, provider_types::ProviderRequest,
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
root = pathlib.Path(os.environ['STDERR_FIXTURE_ROOT'])
case = os.environ['STDERR_FIXTURE_CASE']
(root / 'pid.tmp').write_text(str(os.getpid()))
os.replace(root / 'pid.tmp', root / 'pid')
value = json.loads(sys.stdin.read())
assert value['message']['content'][0]['text'] == 'fixture prompt'
if case == 'diagnostic':
    sys.stderr.write('  fixture startup diagnostic\n')
    sys.stderr.flush()
    sys.exit(1)
if case == 'success':
    print(json.dumps({'type':'assistant','message':{'content':[{'type':'text','text':'fixture response'}]}}), flush=True)
    print(json.dumps({'type':'result'}), flush=True)
    sys.exit(0)
child = os.fork()
if child == 0:
    os.close(0)
    os.close(1)
    (root / 'descendant.tmp').write_text(str(os.getpid()))
    os.replace(root / 'descendant.tmp', root / 'descendant')
    while not (root / 'release').exists():
        time.sleep(0.01)
    os.set_blocking(2, False)
    while True:
        try:
            os.write(2, b'fixture stderr pressure\n' * 128)
        except BrokenPipeError:
            (root / 'broken-pipe.tmp').write_text('closed')
            os.replace(root / 'broken-pipe.tmp', root / 'broken-pipe')
            os._exit(0)
        except BlockingIOError:
            pass
        time.sleep(0.01)
while not (root / 'descendant').exists():
    time.sleep(0.01)
if case == 'early-error':
    sys.exit(1)
if case == 'stream-drop':
    print(json.dumps({'type':'system','session_id':'fixture','model':'fixture'}), flush=True)
time.sleep(60)
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
        "coven-cli-stderr-{}-{case}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("fake.py"), FAKE).unwrap();
    let launcher = root.join("claude");
    fs::write(
        &launcher,
        "#!/bin/sh\nexec \"$STDERR_FIXTURE_PYTHON\" \"$STDERR_FIXTURE_ROOT/fake.py\" \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    let log = fs::File::create(root.join("worker.log")).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "stderr_fixture_worker",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("HOME", &root)
        .env("USERPROFILE", &root)
        .env("COVEN_CODE_HOME", &root)
        .env("PATH", &root)
        .env("COVEN_CODE_CLAUDE_BIN", &launcher)
        .env("STDERR_FIXTURE_ROOT", &root)
        .env("STDERR_FIXTURE_CASE", case)
        .env("STDERR_FIXTURE_PYTHON", &python)
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
fn startup_drop_closes_stderr_reader() {
    run_case("startup-drop");
}
#[test]
fn unpolled_stream_drop_closes_stderr_reader() {
    run_case("stream-drop");
}
#[test]
fn returned_startup_error_closes_stderr_reader() {
    run_case("early-error");
}
#[test]
fn startup_diagnostic_is_preserved() {
    run_case("diagnostic");
}
#[test]
fn normal_response_is_preserved() {
    run_case("success");
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
    let python = std::env::var_os("STDERR_FIXTURE_PYTHON").unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = tokio::process::Command::new(&python).args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]),0)\nexcept ProcessLookupError: sys.exit(0)\nsys.exit(1)", &pid]).status().await.unwrap();
            if status.success() { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("provider child survived drop before group cleanup");
}

async fn wait_for_file(root: &Path, name: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root.join(name).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("missing fixture marker: {name}"));
}

#[test]
#[ignore = "invoked by isolated parent drivers"]
fn stderr_fixture_worker() {
    let root = PathBuf::from(std::env::var_os("STDERR_FIXTURE_ROOT").unwrap());
    let case = std::env::var("STDERR_FIXTURE_CASE").unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(async {
        let provider = ClaudeCliProvider::new();
        let request = ProviderRequest {
            model: "fixture".into(), messages: vec![Message::user("fixture prompt")],
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
                    _ = wait_for_file(&root, "descendant") => {}
                }
            }
        } else if case == "stream-drop" {
            let stream = tokio::time::timeout(Duration::from_secs(3), provider.create_message_stream(request)).await.unwrap().unwrap();
            drop(stream);
        } else if case == "success" {
            let response = tokio::time::timeout(Duration::from_secs(3), provider.create_message(request)).await.unwrap().unwrap();
            assert!(matches!(response.content.as_slice(), [ContentBlock::Text { text }] if text == "fixture response"));
        } else {
            let result = tokio::time::timeout(Duration::from_secs(3), provider.create_message_stream(request)).await.unwrap();
            let error = match result { Ok(_) => panic!("expected startup failure"), Err(error) => error };
            let expected = if case == "diagnostic" { "claude CLI exited without producing output: fixture startup diagnostic" } else { "claude CLI exited without producing output" };
            assert!(matches!(error, ProviderError::Other { message, .. } if message == expected));
        }
        wait_for_pid(&root).await;
        if case != "success" && case != "diagnostic" {
            wait_for_file(&root, "descendant").await;
            let descendant = fs::read_to_string(root.join("descendant")).unwrap().parse::<u32>().unwrap();
            assert!(descendant >= 2);
            assert_child_gone(&root).await;
            fs::write(root.join("release"), "release").unwrap();
            wait_for_file(&root, "broken-pipe").await;
            assert_eq!(fs::read_to_string(root.join("broken-pipe")).unwrap(), "closed");
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
    std::thread::sleep(Duration::from_secs(60));
}
