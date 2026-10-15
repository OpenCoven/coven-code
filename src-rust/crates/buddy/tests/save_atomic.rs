#![cfg(unix)]

use claurst_buddy::{save_companion_soul, CompanionSoul};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn failed_save_preserves_companion() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("companion.json");
    save_companion_soul(
        directory.path(),
        &CompanionSoul {
            name: "original".into(),
            personality: "keep me".into(),
            hatched_at: chrono::Utc::now(),
        },
    )
    .unwrap();
    let original = std::fs::read(&path).unwrap();

    // Only this child inherits the write limit and ignored SIGXFSZ. Both common
    // shell units (512 and 1024 bytes) are smaller than the replacement companion.
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
            "limited_companion_writer",
            "--ignored",
            "--nocapture",
        ])
        .env("COVEN_TEST_COMPANION_DIR", directory.path())
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
                panic!("bounded companion fixture failed: {result:?}");
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
        "failed staging must leave only the companion and completion marker"
    );
    let after = std::fs::read(&path).unwrap();
    assert!(
        after == original.as_slice(),
        "failed save changed original companion: {} bytes before, {} after",
        original.len(),
        after.len()
    );
}

#[test]
#[ignore = "invoked by the bounded file-size-limit driver"]
fn limited_companion_writer() {
    let directory = std::path::PathBuf::from(std::env::var_os("COVEN_TEST_COMPANION_DIR").unwrap());
    let soul = CompanionSoul {
        name: "replacement".into(),
        personality: "x".repeat(16_384),
        hatched_at: chrono::Utc::now(),
    };
    let error = save_companion_soul(&directory, &soul).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::FileTooLarge
    );
    std::fs::write(directory.join("companion.complete"), b"file-size-limit").unwrap();
}

#[test]
fn existing_save_preserves_symlinks_and_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for linked in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("companion.json");
        let target = if linked {
            directory.path().join("actual.json")
        } else {
            path.clone()
        };
        std::fs::write(&target, "original").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        if linked {
            symlink("actual.json", &path).unwrap();
        }
        let soul = CompanionSoul {
            name: "new".into(),
            personality: "retained identity".into(),
            hatched_at: chrono::Utc::now(),
        };
        save_companion_soul(directory.path(), &soul).unwrap();
        let loaded = claurst_buddy::load_companion_soul(directory.path()).unwrap();
        assert_eq!(loaded.name, soul.name);
        assert_eq!(loaded.personality, soul.personality);
        assert_eq!(loaded.hatched_at, soul.hatched_at);
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink(),
            linked
        );
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            if linked { 2 } else { 1 }
        );
    }
}

#[test]
fn missing_save_preserves_creation_and_dangling_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for linked in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        let reference = directory.path().join("reference");
        std::fs::write(&reference, "reference").unwrap();
        let mode = std::fs::metadata(reference).unwrap().permissions().mode() & 0o777;
        let path = config.join("companion.json");
        if linked {
            std::fs::create_dir(&config).unwrap();
            symlink("actual.json", &path).unwrap();
        }
        let soul = CompanionSoul {
            name: "new".into(),
            personality: "new identity".into(),
            hatched_at: chrono::Utc::now(),
        };
        save_companion_soul(&config, &soul).unwrap();
        let loaded = claurst_buddy::load_companion_soul(&config).unwrap();
        assert_eq!(loaded.name, soul.name);
        assert_eq!(loaded.personality, soul.personality);
        assert_eq!(loaded.hatched_at, soul.hatched_at);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            mode
        );
        assert_eq!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink(),
            linked
        );
        assert_eq!(
            std::fs::read_dir(&config).unwrap().count(),
            if linked { 2 } else { 1 }
        );
    }
}

#[test]
fn existing_save_checks_write_access() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("companion.json");
    std::fs::write(&path, "original").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    // A privileged test runner can bypass mode bits; inspect the OS decision first.
    if std::fs::OpenOptions::new().write(true).open(&path).is_ok() {
        eprintln!("write-access rejection unavailable for privileged runner");
        return;
    }
    let soul = CompanionSoul {
        name: "new".into(),
        personality: "new".into(),
        hatched_at: chrono::Utc::now(),
    };
    let error = save_companion_soul(directory.path(), &soul).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
