//! Preserve global extension values through real load/save APIs and layering.
use claurst_core::config::Settings;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_case(mode: &str) {
    let root = tempfile::tempdir().unwrap();
    let engine = root.path().join("engine");
    let shared = root.path().join("shared");
    let project = root.path().join("project");
    std::fs::create_dir_all(&engine).unwrap();
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(project.join(".coven-code")).unwrap();
    let original = serde_json::json!({
        "config": {"model": "global-model"},
        "futureObject": {"mode": "on", "levels": [1, "two", null]},
        "futureSwitch": false, "futureNull": null,
        "futureToken": "synthetic-secret-457", "sharedName": "global"
    });
    std::fs::write(
        engine.join("settings.json"),
        serde_json::to_vec(&original).unwrap(),
    )
    .unwrap();
    std::fs::write(shared.join("settings.json"), r#"{"fromShared":true}"#).unwrap();
    std::fs::write(
        project.join(".coven-code/settings.json"),
        r#"{
        "config":{"model":"project-model"},
        "fromRepository":{"command":"must not persist"},
        "sharedName":"repository", "futureSwitch":true
    }"#,
    )
    .unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "settings_unknown_keys_worker",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root.path())
        .env("USERPROFILE", root.path())
        .env("COVEN_CODE_HOME", &engine)
        .env("COVEN_HOME", &shared)
        .env("SETTINGS_UNKNOWN_MODE", mode)
        .current_dir(&project)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
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
                let _ = child.wait();
                let _ = reader.join();
                panic!("bounded settings worker failed: {other:?}");
            }
        }
    };
    let diagnostics = reader.join().unwrap();
    assert!(status.success(), "{mode}: {status}: {diagnostics}");
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(engine.join("settings.json")).unwrap()).unwrap();
    for key in [
        "futureObject",
        "futureSwitch",
        "futureNull",
        "futureToken",
        "sharedName",
    ] {
        assert_eq!(
            saved.get(key),
            original.get(key),
            "{mode} lost or replaced {key}"
        );
    }
    assert!(
        saved.get("fromRepository").is_none(),
        "repository key persisted"
    );
    assert!(
        saved.get("fromShared").is_none(),
        "shared-only key persisted"
    );
    assert_eq!(saved["lastSeenVersion"], "0.9.0-fixture");
    assert_eq!(
        saved["config"]["model"],
        if mode == "hierarchical" {
            "project-model"
        } else {
            "global-model"
        }
    );
}

#[test]
fn sync_settings_save_preserves_global_extension_values() {
    run_case("sync");
}
#[test]
fn async_settings_save_preserves_global_extension_values() {
    run_case("async");
}
#[test]
fn hierarchical_save_preserves_only_global_extension_values() {
    run_case("hierarchical");
}

#[test]
#[ignore = "owned-home worker invoked by bounded parent"]
fn settings_unknown_keys_worker() {
    let mode = std::env::var("SETTINGS_UNKNOWN_MODE").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut settings = if mode == "sync" {
        Settings::load_sync().unwrap()
    } else if mode == "hierarchical" {
        runtime.block_on(Settings::load_hierarchical(
            &std::env::current_dir().unwrap(),
        ))
    } else {
        runtime.block_on(Settings::load()).unwrap()
    };
    assert!(!format!("{settings:?}").contains("synthetic-secret-457"));
    settings.last_seen_version = Some("0.9.0-fixture".into());
    if mode == "sync" {
        settings.save_sync().unwrap();
    } else {
        runtime.block_on(settings.save()).unwrap();
    }
}
