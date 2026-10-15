use claurst_core::Settings;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn settings_loaders_report_parse_errors() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "settings_parse_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("COVEN_CODE_HOME", directory.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            result => {
                let _ = child.kill();
                child.wait().unwrap();
                panic!("settings fixture failed: {result:?}");
            }
        }
    };
    assert!(status.success(), "settings fixture: {status}");
    assert_eq!(
        std::fs::read(directory.path().join("complete")).unwrap(),
        b"checked"
    );
}

#[test]
#[ignore = "invoked by bounded driver with an isolated settings directory"]
fn settings_parse_fixture() {
    let directory = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(Settings::config_dir(), directory);
    let path = Settings::global_settings_path();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        assert!(Settings::load().await.unwrap().config.model.is_none());
        assert!(Settings::load_sync().unwrap().config.model.is_none());
        assert!(!path.exists(), "missing settings must not create a file");

        let valid = r#"{"config":{"model":"fixture-model"},"future_setting":true}"#;
        std::fs::write(&path, valid).unwrap();
        assert_eq!(
            Settings::load().await.unwrap().config.model.as_deref(),
            Some("fixture-model")
        );
        assert_eq!(
            Settings::load_sync().unwrap().config.model.as_deref(),
            Some("fixture-model")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), valid);

        for malformed in ["{not json", r#"{"config":{"model":42}}"#, "null"] {
            std::fs::write(&path, malformed).unwrap();
            let sync = Settings::load_sync();
            let asynchronous = Settings::load().await;
            assert!(sync.is_err(), "sync loader silently accepted {malformed}");
            assert!(
                asynchronous.is_err(),
                "async loader silently accepted {malformed}"
            );
            for error in [sync.unwrap_err(), asynchronous.unwrap_err()] {
                assert!(
                    error.to_string().contains(&path.display().to_string()),
                    "{error}"
                );
                assert!(
                    error.downcast_ref::<serde_json::Error>().is_some(),
                    "{error:#}"
                );
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);
        }
    });
    std::fs::write(directory.join("complete"), b"checked").unwrap();
}
