#![cfg(unix)]

use claurst_core::{history, types::Message, Settings};
use std::io::Read;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_writer(home: &Path, mode: &str) {
    let mut command = if mode == "limited" {
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' XFSZ; ulimit -f 1 || exit 90; exec \"$@\"",
                "fixture",
            ])
            .arg(std::env::current_exe().unwrap());
        command
    } else {
        Command::new(std::env::current_exe().unwrap())
    };
    let mut child = command
        .args(["--exact", "history_writer", "--ignored", "--nocapture"])
        .env_clear()
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("COVEN_CODE_HOME", home)
        .env("COVEN_HOME", home.join("coven"))
        .env("COVEN_CAVE_HOME", home.join("cave"))
        .env("HISTORY_SAVE_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Regular-file diagnostics would also be subject to RLIMIT_FSIZE.
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
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => {
                let _ = child.kill();
                child.wait().unwrap();
                let _ = diagnostics.join();
                panic!("bounded history writer failed: {other:?}");
            }
        }
    };
    let diagnostics = diagnostics.join().unwrap();
    assert!(status.success(), "{status}: {diagnostics}");
    assert_eq!(std::fs::read(home.join("completed")).unwrap(), b"complete");
}

#[test]
fn failed_history_save_preserves_original() {
    let temp = tempfile::tempdir().unwrap();
    let sessions = temp.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let path = sessions.join("fixture-session.json");
    let mut original = history::ConversationSession::new("original-model".into());
    original.id = "fixture-session".into();
    original.messages.push(Message::user("retained original"));
    let bytes = serde_json::to_vec_pretty(&original).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    run_writer(temp.path(), "limited");
    let after = std::fs::read(&path).unwrap();
    assert!(
        after == bytes,
        "failed save changed original: {} bytes before, {} after",
        bytes.len(),
        after.len()
    );
    assert_eq!(std::fs::read_dir(sessions).unwrap().count(), 1);
}

#[test]
fn history_save_preserves_creation_links_and_permission_bits() {
    for case in [
        "existing",
        "live-link",
        "new",
        "dangling-link",
        "missing-parent",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let home = if case == "missing-parent" {
            temp.path().join("nested")
        } else {
            temp.path().to_path_buf()
        };
        let sessions = home.join("sessions");
        let path = sessions.join("fixture-session.json");
        let linked = case.contains("link");
        let target = if linked {
            sessions.join("actual.json")
        } else {
            path.clone()
        };
        if case != "missing-parent" {
            std::fs::create_dir_all(&sessions).unwrap();
        }
        if matches!(case, "existing" | "live-link") {
            std::fs::write(&target, b"original").unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        if linked {
            symlink("actual.json", &path).unwrap();
        }
        run_writer(&home, "normal");
        let bytes = std::fs::read(&target).unwrap();
        let session: history::ConversationSession = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(session.id, "fixture-session");
        assert_eq!(session.model, "replacement");
        assert_eq!(session.messages[0].get_all_text(), "replacement message");
        assert_eq!(bytes, serde_json::to_vec_pretty(&session).unwrap());
        if matches!(case, "existing" | "live-link") {
            assert_eq!(
                std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o640
            );
        } else {
            let control = home.join("mode-control");
            std::fs::write(&control, b"control").unwrap();
            assert_eq!(
                std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                std::fs::metadata(&control).unwrap().permissions().mode() & 0o777
            );
        }
        assert_eq!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink(),
            linked
        );
        assert_eq!(
            std::fs::read_dir(sessions).unwrap().count(),
            if linked { 2 } else { 1 }
        );
    }
}

#[test]
fn history_save_preserves_write_access_checks() {
    let temp = tempfile::tempdir().unwrap();
    let sessions = temp.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let path = sessions.join("fixture-session.json");
    std::fs::write(&path, b"original").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o440)).unwrap();
    if std::fs::OpenOptions::new().write(true).open(&path).is_ok() {
        eprintln!("Skipping readonly rejection: this process can write mode0440");
        return;
    }
    run_writer(temp.path(), "readonly");
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(sessions).unwrap().count(), 1);
}

#[test]
#[ignore = "owned history writer invoked by bounded parent"]
fn history_writer() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(Settings::config_dir(), home);
    let mode = std::env::var("HISTORY_SAVE_MODE").unwrap();
    let mut session = history::ConversationSession::new("replacement".into());
    session.id = "fixture-session".into();
    session.messages.push(Message::user(if mode == "limited" {
        "x".repeat(16_384)
    } else {
        "replacement message".into()
    }));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(history::save_session(&session));
    if mode == "limited" || mode == "readonly" {
        assert_eq!(
            result
                .unwrap_err()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            if mode == "limited" {
                std::io::ErrorKind::FileTooLarge
            } else {
                std::io::ErrorKind::PermissionDenied
            }
        );
    } else {
        result.unwrap();
        assert_eq!(
            std::fs::read(home.join("sessions/fixture-session.json")).unwrap(),
            serde_json::to_vec_pretty(&session).unwrap()
        );
    }
    std::fs::write(home.join("completed"), b"complete").unwrap();
}
