use claurst_core::Settings;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn settings_reads_distinguish_absence_from_errors() {
    let cases = [
        "not-directory",
        "missing-parent",
        "directory",
        "utf8",
        "valid",
    ];
    #[cfg(unix)]
    let cases = [cases.as_slice(), &["live-link", "dangling-link"]].concat();
    for case in cases {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        match case {
            "not-directory" => std::fs::write(&home, b"unchanged parent file").unwrap(),
            "missing-parent" => (),
            _ => std::fs::create_dir(&home).unwrap(),
        }
        let path = home.join("settings.json");
        match case {
            "directory" => std::fs::create_dir(&path).unwrap(),
            "utf8" => std::fs::write(&path, [0xff]).unwrap(),
            "valid" => std::fs::write(&path, br#"{"config":{"model":"read-fixture"}}"#).unwrap(),
            _ => (),
        }
        #[cfg(unix)]
        if matches!(case, "live-link" | "dangling-link") {
            if case == "live-link" {
                std::fs::write(
                    home.join("target.json"),
                    br#"{"config":{"model":"read-fixture"}}"#,
                )
                .unwrap();
            }
            std::os::unix::fs::symlink("target.json", &path).unwrap();
        }
        let log_path = temp.path().join("log");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "settings_read_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("COVEN_CODE_HOME", &home)
            .env_remove("COVEN_CODE_TEST_HOME")
            .env("SETTINGS_READ_CASE", case)
            .env("SETTINGS_READ_COMPLETE", temp.path().join("complete"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log_path).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{case}: child wait failed: {result:?}");
                }
            }
        };
        assert!(
            status.success(),
            "{case}: {}",
            std::fs::read_to_string(log_path).unwrap()
        );
        assert_eq!(
            std::fs::read(temp.path().join("complete")).unwrap(),
            b"checked"
        );
        match case {
            "not-directory" => assert_eq!(std::fs::read(&home).unwrap(), b"unchanged parent file"),
            "missing-parent" => assert!(!home.exists()),
            "directory" => assert!(path.is_dir()),
            "utf8" => assert_eq!(std::fs::read(&path).unwrap(), [0xff]),
            "valid" => assert_eq!(
                std::fs::read(&path).unwrap(),
                br#"{"config":{"model":"read-fixture"}}"#
            ),
            #[cfg(unix)]
            "live-link" | "dangling-link" => {
                assert_eq!(
                    std::fs::read_link(&path).unwrap(),
                    std::path::Path::new("target.json")
                );
                if case == "live-link" {
                    assert_eq!(
                        std::fs::read(home.join("target.json")).unwrap(),
                        br#"{"config":{"model":"read-fixture"}}"#
                    );
                } else {
                    assert!(!home.join("target.json").exists());
                }
            }
            _ => unreachable!(),
        }
    }
}

#[test]
#[ignore = "bounded child with an owned settings path"]
fn settings_read_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(Settings::config_dir(), home);
    let case = std::env::var("SETTINGS_READ_CASE").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let asynchronous = runtime.block_on(Settings::load());
    let synchronous = Settings::load_sync();
    for result in [asynchronous, synchronous] {
        match case.as_str() {
            "missing-parent" | "dangling-link" => assert!(result.unwrap().config.model.is_none()),
            "valid" | "live-link" => assert_eq!(
                result.unwrap().config.model.as_deref(),
                Some("read-fixture")
            ),
            _ => {
                let expected =
                    std::fs::read_to_string(Settings::global_settings_path()).unwrap_err();
                assert_ne!(expected.kind(), std::io::ErrorKind::NotFound);
                #[cfg(unix)]
                if case == "not-directory" {
                    assert_eq!(expected.kind(), std::io::ErrorKind::NotADirectory);
                }
                let error = result.expect_err("unreadable settings must not silently default");
                let actual = error.downcast_ref::<std::io::Error>().unwrap();
                assert_eq!(actual.kind(), expected.kind());
                assert_eq!(actual.raw_os_error(), expected.raw_os_error());
            }
        }
    }
    std::fs::write(
        std::env::var_os("SETTINGS_READ_COMPLETE").unwrap(),
        b"checked",
    )
    .unwrap();
}
