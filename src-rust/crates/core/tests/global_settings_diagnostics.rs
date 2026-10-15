use claurst_core::Settings;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn global_settings_failures_warn_without_changing_fallbacks() {
    for case in ["syntax", "data", "utf8", "directory", "valid", "missing"] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("engine");
        let shared = temp.path().join("coven");
        let project = temp.path().join("project");
        std::fs::create_dir_all(project.join(".coven-code")).unwrap();
        std::fs::create_dir(&shared).unwrap();
        std::fs::write(shared.join("settings.json"), br#"{"model":"shared-model"}"#).unwrap();
        std::fs::write(
            project.join(".coven-code/settings.json"),
            br#"{"config":{"max_tokens":1234}}"#,
        )
        .unwrap();
        let path = home.join("settings.json");
        std::fs::create_dir(&home).unwrap();
        match case {
            "syntax" => std::fs::write(&path, b"{").unwrap(),
            "data" => std::fs::write(&path, br#"{"config":{"model":782346981235670123}}"#).unwrap(),
            "utf8" => std::fs::write(&path, [0xff]).unwrap(),
            "directory" => std::fs::create_dir(&path).unwrap(),
            "valid" => std::fs::write(
                &path,
                br#"{"config":{"model":"global-model"},"future_key":42}"#,
            )
            .unwrap(),
            _ => (),
        }
        let before = std::fs::read(&path).ok();
        if case == "data" {
            // A raw serde error includes this value; the diagnostic must not.
            let raw_error =
                serde_json::from_slice::<Settings>(before.as_ref().unwrap()).unwrap_err();
            assert!(raw_error.to_string().contains("782346981235670123"));
        }
        let log_path = temp.path().join("diagnostics");
        let log = std::fs::File::create(&log_path).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "global_settings_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("HOME", temp.path())
            .env("USERPROFILE", temp.path())
            .env("COVEN_HOME", &shared)
            .env("COVEN_CODE_HOME", &home)
            .env_remove("COVEN_CODE_TEST_HOME")
            .env("GLOBAL_FIXTURE_PROJECT", &project)
            .env("GLOBAL_FIXTURE_CASE", case)
            .env("GLOBAL_FIXTURE_COMPLETED", temp.path().join("completed"))
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
        if matches!(case, "valid" | "missing") {
            assert!(!log.contains("WARN"), "{log}");
        } else {
            assert!(log.contains("WARN"), "{case}: missing warning: {log}");
            assert!(log.contains(&path.display().to_string()), "{log}");
            assert!(log.contains("using defaults"), "{log}");
            if matches!(case, "syntax" | "data") {
                assert!(log.contains("Invalid global settings"), "{log}");
                assert!(
                    log.contains("category=") && log.contains("line=") && log.contains("column="),
                    "{log}"
                );
            } else {
                assert!(
                    log.contains("Unreadable global settings") && log.contains("error="),
                    "{log}"
                );
            }
        }
    }
}

#[test]
#[ignore = "isolated home and diagnostic subscriber worker"]
fn global_settings_worker() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
    let case = std::env::var("GLOBAL_FIXTURE_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(Settings::config_dir(), home);
    let project = std::path::PathBuf::from(std::env::var_os("GLOBAL_FIXTURE_PROJECT").unwrap());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let loaded = runtime.block_on(Settings::load_hierarchical(&project));
    assert_eq!(
        loaded.config.model.as_deref(),
        Some(if case == "valid" {
            "global-model"
        } else {
            "shared-model"
        })
    );
    assert_eq!(loaded.config.max_tokens, Some(1234));
    std::fs::write(
        std::env::var_os("GLOBAL_FIXTURE_COMPLETED").unwrap(),
        b"complete",
    )
    .unwrap();
}
