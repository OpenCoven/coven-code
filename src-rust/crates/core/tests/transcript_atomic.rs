#![cfg(unix)]

use claurst_core::session_storage::{load_transcript, make_user_entry, truncate_after};
use claurst_core::types::Message;
use std::io::Read;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TRANSCRIPT: &str = "COVEN_TEST_ATOMIC_TRANSCRIPT";

fn transcript_contents() -> String {
    let mut original = String::new();
    for (id, content) in [("keep", "x".repeat(16_384)), ("remove", "discard".into())] {
        let mut message = Message::user(content);
        message.uuid = Some(id.into());
        let entry = make_user_entry(message, id, None, "fixture", "/fixture");
        original.push_str(&serde_json::to_string(&entry).unwrap());
        original.push('\n');
    }
    original
}

#[test]
fn failed_transcript_rewrite_preserves_original() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transcript.jsonl");
    let original = transcript_contents();
    std::fs::write(&path, &original).unwrap();

    // Only this child inherits the write limit and ignored SIGXFSZ. Both common
    // shell units (512 and 1024 bytes) are smaller than the retained message.
    // exec replaces the shell, so killing/reaping the child leaves no descendant.
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            "trap '' XFSZ; ulimit -f 1 || exit 90; exec \"$@\"",
            "fixture",
        ])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "limited_transcript_writer",
            "--ignored",
            "--nocapture",
        ])
        .env(TRANSCRIPT, &path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Regular-file stdout/stderr would also inherit RLIMIT_FSIZE. Drain a pipe
    // for panic diagnostics instead, so only fixture filesystem writes are capped.
    let mut stderr = child.stderr.take().unwrap();
    let diagnostics = std::thread::spawn(move || {
        let mut text = String::new();
        stderr.read_to_string(&mut text).unwrap();
        text
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => {
                let _ = child.kill();
                child.wait().unwrap();
                let _ = diagnostics.join();
                panic!("bounded transcript fixture failed: {result:?}");
            }
        }
    };
    let diagnostics = diagnostics.join().unwrap();
    assert!(
        status.success(),
        "write-limit fixture failed: {status}: {diagnostics}"
    );
    assert_eq!(
        std::fs::read(path.with_extension("complete")).unwrap(),
        b"file-size-limit"
    );
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        2,
        "failed staging must leave only the transcript and completion marker"
    );
    let after = std::fs::read(&path).unwrap();
    assert!(
        after == original.as_bytes(),
        "failed rewrite changed original transcript: {} bytes before, {} after",
        original.len(),
        after.len()
    );
}

#[test]
#[ignore = "invoked by the bounded file-size-limit driver"]
fn limited_transcript_writer() {
    let path = std::path::PathBuf::from(std::env::var_os(TRANSCRIPT).unwrap());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(truncate_after(&path, "remove"));
    match result {
        Err(claurst_core::ClaudeError::Io(error)) => {
            assert_eq!(error.kind(), std::io::ErrorKind::FileTooLarge);
        }
        other => panic!("expected file-size-limit I/O failure, got {other:?}"),
    }
    std::fs::write(path.with_extension("complete"), b"file-size-limit").unwrap();
}

#[tokio::test]
async fn transcript_rewrite_preserves_symlink_and_permissions() {
    for use_symlink in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.jsonl");
        std::fs::write(&target, transcript_contents()).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        let path = if use_symlink {
            let link = directory.path().join("link.jsonl");
            symlink("target.jsonl", &link).unwrap();
            link
        } else {
            target.clone()
        };
        truncate_after(&path, "remove").await.unwrap();
        let entries = load_transcript(&target).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].uuid(), Some("keep"));
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        if use_symlink {
            assert!(std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            if use_symlink { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn transcript_rewrite_preserves_missing_target_creation() {
    for use_symlink in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("missing.jsonl");
        let path = if use_symlink {
            let link = directory.path().join("link.jsonl");
            symlink("missing.jsonl", &link).unwrap();
            link
        } else {
            target.clone()
        };
        truncate_after(&path, "absent").await.unwrap();
        assert!(std::fs::read(&target).unwrap().is_empty());
        if use_symlink {
            assert!(std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
        }
    }
}
