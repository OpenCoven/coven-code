#![cfg(unix)]

use claurst_core::{execute_import, ImportPaths, ImportSelection, Settings};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn import_reports_rollback_failure() {
    for fail_rollback in [false, true] {
        let original = if fail_rollback {
            "x".repeat(8192)
        } else {
            "original".to_string()
        };
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join(".claude")).unwrap();
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::write(root.join(".claude/CLAUDE.md"), "new instructions").unwrap();
        std::fs::write(root.join(".claude/settings.json"), r#"{"theme":"dark"}"#).unwrap();
        let path = root.join("target/CLAUDE.md");
        std::fs::write(&path, &original).unwrap();
        // The serialized settings exceed either shell's 512/1024-byte limit.
        std::fs::write(root.join("target/settings.json"), "{}").unwrap();

        // Only this child inherits the write limit and ignored SIGXFSZ. Both common
        // shell units (512 and 1024 bytes) are smaller than the serialized settings.
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
                "limited_import_failure_writer",
                "--ignored",
                "--nocapture",
            ])
            .env("HOME", root)
            .env(
                "COVEN_TEST_FAIL_ROLLBACK",
                if fail_rollback { "1" } else { "0" },
            )
            .env("COVEN_CODE_HOME", root.join("target"))
            .env_remove("COVEN_CODE_TEST_HOME")
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
                    panic!("bounded import fixture failed: {result:?}");
                }
            }
        };
        let diagnostics = diagnostics.join().unwrap();
        assert!(
            status.success(),
            "write-limit fixture failed: {status}: {diagnostics}"
        );
        assert_eq!(
            std::fs::read(root.join("complete")).unwrap(),
            b"file-size-limit"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            if fail_rollback {
                b"new instructions".as_slice()
            } else {
                original.as_bytes()
            }
        );
        assert_eq!(
            std::fs::read(root.join("target/settings.json")).unwrap(),
            b"{}"
        );
    }
}

#[test]
#[ignore = "invoked by bounded file-size-limit driver"]
fn limited_import_failure_writer() {
    let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let paths = ImportPaths::detect();
    assert_eq!(paths.source_claude_md, root.join(".claude/CLAUDE.md"));
    assert_eq!(paths.target_claude_md, root.join("target/CLAUDE.md"));
    assert_eq!(Settings::config_dir(), root.join("target"));
    let error = execute_import(ImportSelection::Both).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::FileTooLarge
    );
    let cause = error.downcast_ref::<std::io::Error>().unwrap().to_string();
    let display = error.to_string();
    if std::env::var("COVEN_TEST_FAIL_ROLLBACK").unwrap() == "1" {
        assert!(display.contains("Import failed:"), "{display}");
        let (import_failure, rollback_failure) = display
            .split_once("; rollback failed for ")
            .expect("both failures must be labeled");
        assert_eq!(import_failure, format!("Import failed: {cause}"));
        let rollback_cause = rollback_failure
            .strip_prefix(&format!("{}: ", paths.target_claude_md.display()))
            .unwrap()
            .strip_suffix("; original file was not restored")
            .unwrap();
        // Atomic staging can attach a different temporary path to each I/O error.
        assert!(!rollback_cause.is_empty());
        assert!(display.contains("rollback failed for"), "{display}");
        assert!(
            display.contains(&paths.target_claude_md.display().to_string()),
            "{display}"
        );
        assert!(
            display.contains("original file was not restored"),
            "{display}"
        );
    } else {
        assert_eq!(display, cause);
    }
    std::fs::write(root.join("complete"), b"file-size-limit").unwrap();
}
