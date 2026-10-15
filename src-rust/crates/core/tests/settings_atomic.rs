#![cfg(unix)]
use claurst_core::config::Settings;
use std::io::Read;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_writer(home: &std::path::Path, api: &str, mode: &str) {
    let limited = mode == "limited";
    let mut command = if limited {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "trap '' XFSZ; ulimit -f 1 || exit 90; exec \"$@\"",
            "fixture",
        ]);
        command.arg(std::env::current_exe().unwrap());
        command
    } else {
        Command::new(std::env::current_exe().unwrap())
    };
    let mut child = command
        .args(["--exact", "settings_writer", "--ignored", "--nocapture"])
        .env("COVEN_CODE_HOME", home)
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("SETTINGS_SAVE_API", api)
        .env("SETTINGS_SAVE_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
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
                panic!("bounded settings writer failed: {other:?}");
            }
        }
    };
    let diagnostics = diagnostics.join().unwrap();
    assert!(status.success(), "{status}: {diagnostics}");
    assert_eq!(std::fs::read(home.join("completed")).unwrap(), b"complete");
}

fn check_failed_save(api: &str) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let original = br#"{"config":{"model":"original"}}"#;
    std::fs::write(&path, original).unwrap();
    run_writer(temp.path(), api, "limited");
    let after = std::fs::read(path).unwrap();
    assert!(
        after == original,
        "{api} failed save changed original: {} bytes before, {} after",
        original.len(),
        after.len()
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
}

#[test]
fn failed_sync_save_preserves_settings() {
    check_failed_save("sync");
}
#[test]
fn failed_async_save_preserves_settings() {
    check_failed_save("async");
}

#[test]
fn settings_save_preserves_links_permissions_and_creation() {
    for api in ["sync", "async"] {
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
            let path = home.join("settings.json");
            let target = if case.contains("link") {
                home.join("actual.json")
            } else {
                path.clone()
            };
            if matches!(case, "existing" | "live-link") {
                std::fs::write(&target, b"{}").unwrap();
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
            }
            if case.contains("link") {
                symlink("actual.json", &path).unwrap();
            }
            run_writer(&home, api, "normal");
            let settings: Settings =
                serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
            assert_eq!(settings.config.model.as_deref(), Some("replacement"));
            let mut expected = Settings::default();
            expected.config.model = Some("replacement".into());
            assert_eq!(
                std::fs::read(&target).unwrap(),
                serde_json::to_vec_pretty(&expected).unwrap()
            );
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
                std::fs::remove_file(control).unwrap();
            }
            if case.contains("link") {
                assert!(std::fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink());
            }
            assert_eq!(
                std::fs::read_dir(&home).unwrap().count(),
                if case.contains("link") { 3 } else { 2 }
            );
        }
    }
}

#[test]
fn settings_save_preserves_write_access_checks() {
    for api in ["sync", "async"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o440)).unwrap();
        if std::fs::OpenOptions::new().write(true).open(&path).is_ok() {
            eprintln!("Skipping readonly rejection: this process can write mode0440");
            continue;
        }
        run_writer(temp.path(), api, "readonly");
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    }
}

#[test]
#[ignore = "owned home writer invoked by bounded parent"]
fn settings_writer() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(Settings::global_settings_path(), home.join("settings.json"));
    let mode = std::env::var("SETTINGS_SAVE_MODE").unwrap();
    let limited = mode == "limited";
    let mut settings = Settings::default();
    settings.config.model = Some(if limited {
        "x".repeat(16_384)
    } else {
        "replacement".into()
    });
    let result = if std::env::var("SETTINGS_SAVE_API").unwrap() == "sync" {
        settings.save_sync()
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(settings.save())
    };
    if limited {
        let error = result.unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::FileTooLarge
        );
    } else if mode == "readonly" {
        assert_eq!(
            result
                .unwrap_err()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
    } else {
        result.unwrap();
    }
    std::fs::write(home.join("completed"), b"complete").unwrap();
}
