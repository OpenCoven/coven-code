use claurst_core::config::SharedSettings;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn shared_settings_failures_warn_without_changing_fallbacks() {
    for case in [
        "syntax",
        "data",
        "utf8",
        "directory",
        "valid",
        "missing",
        "no-home",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("coven");
        let path = home.join("settings.json");
        if case != "no-home" {
            std::fs::create_dir(&home).unwrap();
        }
        match case {
            "syntax" => std::fs::write(&path, b"{").unwrap(),
            "data" => std::fs::write(&path, br#"{"model":782346981235670123}"#).unwrap(),
            "utf8" => std::fs::write(&path, [0xff]).unwrap(),
            "directory" => std::fs::create_dir(&path).unwrap(),
            "valid" => std::fs::write(&path, br#"{"model":"shared-model","theme":"dark","permission_mode":"plan","future_key":42}"#).unwrap(),
            _ => (),
        }
        let before = std::fs::read(&path).ok();
        if case == "data" {
            // A raw serde error includes this value; the diagnostic must not.
            let raw_error =
                serde_json::from_slice::<SharedSettings>(before.as_ref().unwrap()).unwrap_err();
            assert!(raw_error.to_string().contains("782346981235670123"));
        }
        let log_path = temp.path().join("diagnostics");
        let log = std::fs::File::create(&log_path).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shared_settings_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("HOME", temp.path())
            .env("USERPROFILE", temp.path())
            .env("COVEN_HOME", &home)
            .env("SHARED_FIXTURE_CASE", case)
            .env("SHARED_FIXTURE_COMPLETED", temp.path().join("completed"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{case} exceeded deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let log = std::fs::read_to_string(log_path).unwrap();
        assert!(status.success(), "{case}: {status}\n{log}");
        assert!(temp.path().join("completed").is_file());
        assert_eq!(std::fs::read(&path).ok(), before);
        assert!(!log.contains("782346981235670123"), "{log}");
        if matches!(case, "valid" | "missing" | "no-home") {
            assert!(!log.contains("WARN"), "{log}");
        } else {
            assert!(log.contains("WARN"), "{case}: missing warning: {log}");
            assert!(log.contains(&path.display().to_string()), "{log}");
            assert!(log.contains("using defaults"), "{log}");
            if matches!(case, "syntax" | "data") {
                assert!(log.contains("Invalid shared settings"), "{log}");
                assert!(
                    log.contains("category=") && log.contains("line=") && log.contains("column="),
                    "{log}"
                );
            } else {
                assert!(
                    log.contains("Unreadable shared settings") && log.contains("error="),
                    "{log}"
                );
            }
        }
    }
}

#[test]
#[ignore = "isolated home and diagnostic subscriber worker"]
fn shared_settings_worker() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
    let case = std::env::var("SHARED_FIXTURE_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_HOME").unwrap());
    assert_eq!(
        claurst_core::coven_shared::coven_home(),
        if case == "no-home" { None } else { Some(home) }
    );
    let loaded = SharedSettings::load();
    if case == "valid" {
        assert_eq!(loaded.model.as_deref(), Some("shared-model"));
        assert_eq!(loaded.theme.as_deref(), Some("dark"));
        assert_eq!(loaded.permission_mode.as_deref(), Some("plan"));
    } else {
        assert!(
            loaded.model.is_none() && loaded.theme.is_none() && loaded.permission_mode.is_none()
        );
    }
    std::fs::write(
        std::env::var_os("SHARED_FIXTURE_COMPLETED").unwrap(),
        b"complete",
    )
    .unwrap();
}
