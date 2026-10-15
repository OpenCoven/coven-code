#![cfg(unix)]

use claurst_core::{build_import_preview, execute_import, ImportPaths, ImportSelection, Settings};
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn import_rejects_unreadable_target_before_writing() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "import_read_fixture", "--ignored", "--nocapture"])
        .env("HOME", directory.path())
        .env("COVEN_CODE_HOME", directory.path().join("target"))
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
                panic!("import fixture failed: {result:?}");
            }
        }
    };
    assert!(status.success(), "import fixture: {status}");
    assert_eq!(
        std::fs::read(directory.path().join("complete")).unwrap(),
        b"checked"
    );
}

#[test]
#[ignore = "invoked by bounded driver with isolated source and target directories"]
fn import_read_fixture() {
    let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let paths = ImportPaths::detect();
    assert_eq!(
        paths.source_settings_json,
        root.join(".claude/settings.json")
    );
    assert_eq!(
        paths.target_settings_json,
        root.join("target/settings.json")
    );
    assert_eq!(Settings::config_dir(), root.join("target"));
    std::fs::create_dir_all(root.join(".claude")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(&paths.source_settings_json, r#"{"theme":"dark"}"#).unwrap();
    std::fs::write(&paths.source_claude_md, "new instructions").unwrap();
    for selection in [ImportSelection::Settings, ImportSelection::Both] {
        let original = b"\xfforiginal settings";
        std::fs::write(&paths.target_settings_json, original).unwrap();
        std::fs::write(&paths.target_claude_md, "original instructions").unwrap();
        let preview = build_import_preview(selection);
        let execution = execute_import(selection);
        assert_eq!(
            std::fs::read(&paths.target_settings_json).unwrap(),
            original,
            "failed settings read must not replace original bytes"
        );
        assert_eq!(
            std::fs::read_to_string(&paths.target_claude_md).unwrap(),
            "original instructions"
        );
        assert!(
            preview.is_err(),
            "preview silently accepted unreadable target"
        );
        assert!(
            execution.is_err(),
            "execution silently accepted unreadable target"
        );
        for error in [preview.unwrap_err(), execution.unwrap_err()] {
            assert!(error
                .to_string()
                .contains(&paths.target_settings_json.display().to_string()));
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::InvalidData
            );
        }
    }
    assert!(build_import_preview(ImportSelection::ClaudeMd).is_ok());
    let result = execute_import(ImportSelection::ClaudeMd).unwrap();
    assert!(result.wrote_claude_md && !result.wrote_settings);
    assert_eq!(
        std::fs::read(&paths.target_settings_json).unwrap(),
        b"\xfforiginal settings"
    );
    std::fs::remove_file(&paths.target_settings_json).unwrap();
    assert!(build_import_preview(ImportSelection::Both).is_ok());
    let result = execute_import(ImportSelection::Both).unwrap();
    assert!(result.wrote_settings && result.wrote_claude_md);
    let mut settings = Settings::load_sync().unwrap();
    assert!(matches!(
        settings.config.theme,
        claurst_core::config::Theme::Dark
    ));
    settings.config.model = Some("preserved-model".into());
    settings.save_sync().unwrap();
    execute_import(ImportSelection::Settings).unwrap();
    assert_eq!(
        Settings::load_sync().unwrap().config.model.as_deref(),
        Some("preserved-model")
    );
    std::fs::write(root.join("complete"), b"checked").unwrap();
}
