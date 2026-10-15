use claurst_core::config::Settings;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn project_settings_failures_are_diagnostic_and_stop_search() {
    for case in ["syntax", "data", "utf8", "directory", "jsonc", "missing"] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let global = home.join("engine");
        let project = temp.path().join("project");
        let nested = project.join("nested");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(home.join("coven")).unwrap();
        std::fs::write(
            global.join("settings.json"),
            r#"{"config":{"model":"global"}}"#,
        )
        .unwrap();
        if case != "missing" {
            std::fs::create_dir_all(project.join(".coven-code")).unwrap();
            std::fs::write(
                project.join(".coven-code/settings.json"),
                r#"{"config":{"model":"ancestor"}}"#,
            )
            .unwrap();
            let config = nested.join(".coven-code");
            std::fs::create_dir(&config).unwrap();
            // A bad .json must not fall through to this valid sibling or ancestor.
            std::fs::write(
                config.join("settings.jsonc"),
                "// comment\n{\"config\":{\"model\":\"jsonc\"}}",
            )
            .unwrap();
            match case {
                "syntax" => std::fs::write(config.join("settings.json"), b"{").unwrap(),
                "data" => std::fs::write(
                    config.join("settings.json"),
                    br#"{"config":{"permission_mode":"PRIVATE_CONFIG_VALUE_DO_NOT_LOG"}}"#,
                )
                .unwrap(),
                "utf8" => std::fs::write(config.join("settings.json"), [0xff]).unwrap(),
                "directory" => std::fs::create_dir(config.join("settings.json")).unwrap(),
                "jsonc" => (),
                _ => unreachable!(),
            }
        }
        let log_path = temp.path().join("diagnostics");
        let log = std::fs::File::create(&log_path).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "project_settings_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("COVEN_HOME", home.join("coven"))
            .env("COVEN_CODE_HOME", &global)
            .env_remove("COVEN_CODE_TEST_HOME")
            .env("SETTINGS_FIXTURE_CASE", case)
            .env("SETTINGS_FIXTURE_ROOT", &nested)
            .current_dir(&nested)
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
        assert!(nested.join("completed").is_file());
        assert!(!log.contains("PRIVATE_CONFIG_VALUE_DO_NOT_LOG"), "{log}");
        if case == "jsonc" || case == "missing" {
            assert!(!log.contains("WARN"), "{case}: {log}");
        } else {
            assert!(log.contains("WARN"), "{case}: missing warning: {log}");
            assert!(
                log.contains(
                    &nested
                        .join(".coven-code/settings.json")
                        .display()
                        .to_string()
                ),
                "{log}"
            );
            assert!(log.contains("ancestor search stopped"), "{log}");
            if case == "syntax" || case == "data" {
                assert!(log.contains("Invalid project settings"), "{log}");
                assert!(
                    log.contains("line=") && log.contains("column=") && log.contains("category="),
                    "{log}"
                );
            } else {
                assert!(
                    log.contains("Unreadable project settings") && log.contains("error="),
                    "{log}"
                );
            }
        }
    }
}

#[tokio::test]
#[ignore = "isolated home and diagnostic subscriber worker"]
async fn project_settings_worker() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
    let root = std::path::PathBuf::from(std::env::var_os("SETTINGS_FIXTURE_ROOT").unwrap());
    let case = std::env::var("SETTINGS_FIXTURE_CASE").unwrap();
    let global = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(
        Settings::global_settings_path(),
        global.join("settings.json")
    );
    let loaded = Settings::load_hierarchical(&root).await;
    assert_eq!(
        loaded.config.model.as_deref(),
        Some(if case == "jsonc" { "jsonc" } else { "global" })
    );
    std::fs::write(root.join("completed"), b"complete").unwrap();
}
