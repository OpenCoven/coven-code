#![cfg(unix)]

use claurst_core::{execute_import, ImportPaths, ImportSelection, Settings};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn failed_import_restores_original_bytes() {
    for original in [
        None,
        Some(&b"original instructions"[..]),
        Some(&b"\xfforiginal instructions"[..]),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join(".claude")).unwrap();
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::write(root.join(".claude/CLAUDE.md"), "new instructions").unwrap();
        std::fs::write(root.join(".claude/settings.json"), r#"{"theme":"dark"}"#).unwrap();
        let path = root.join("target/CLAUDE.md");
        if let Some(original) = original {
            std::fs::write(&path, original).unwrap();
        }
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
                "limited_import_writer",
                "--ignored",
                "--nocapture",
            ])
            .env("HOME", root)
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
        match original {
            Some(original) => assert_eq!(std::fs::read(&path).unwrap(), original),
            None => assert!(!path.exists()),
        }
        assert_eq!(
            std::fs::read(root.join("target/settings.json")).unwrap(),
            b"{}"
        );
    }
}

#[test]
#[ignore = "invoked by bounded file-size-limit driver"]
fn limited_import_writer() {
    let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let paths = ImportPaths::detect();
    assert_eq!(paths.source_claude_md, root.join(".claude/CLAUDE.md"));
    assert_eq!(paths.target_claude_md, root.join("target/CLAUDE.md"));
    assert_eq!(Settings::config_dir(), root.join("target"));
    // A backup read error must stop before either target is written.
    let saved = root.join("saved-document");
    let had_original = paths.target_claude_md.exists();
    if had_original {
        std::fs::rename(&paths.target_claude_md, &saved).unwrap();
    }
    std::fs::create_dir(&paths.target_claude_md).unwrap();
    std::fs::write(paths.target_claude_md.join("sentinel"), b"unchanged").unwrap();
    let count = std::fs::read_dir(Settings::config_dir()).unwrap().count();
    let backup_error = execute_import(ImportSelection::Both).unwrap_err();
    assert!(backup_error
        .to_string()
        .contains("Failed to back up import target"));
    assert!(backup_error
        .to_string()
        .contains(&paths.target_claude_md.display().to_string()));
    assert!(backup_error.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(
        std::fs::read(paths.target_claude_md.join("sentinel")).unwrap(),
        b"unchanged"
    );
    assert_eq!(std::fs::read(&paths.target_settings_json).unwrap(), b"{}");
    assert_eq!(
        std::fs::read_dir(Settings::config_dir()).unwrap().count(),
        count
    );
    std::fs::remove_file(paths.target_claude_md.join("sentinel")).unwrap();
    std::fs::remove_dir(&paths.target_claude_md).unwrap();
    if had_original {
        std::fs::rename(saved, &paths.target_claude_md).unwrap();
    }
    let error = execute_import(ImportSelection::Both).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::FileTooLarge
    );
    std::fs::write(root.join("complete"), b"file-size-limit").unwrap();
}
