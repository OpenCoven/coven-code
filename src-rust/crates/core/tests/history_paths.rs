use claurst_core::history::{delete_session, load_session, save_session, ConversationSession};
use std::process::Command;
use std::time::{Duration, Instant};

const OPERATION: &str = "COVEN_TEST_HISTORY_OPERATION";
const ABSOLUTE: &str = "COVEN_TEST_HISTORY_ABSOLUTE";

fn run_case(operation: &str) {
    for absolute in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "history_path_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("COVEN_CODE_HOME", directory.path())
            .env_remove("COVEN_CODE_TEST_HOME")
            .env(OPERATION, operation)
            .env(ABSOLUTE, if absolute { "yes" } else { "no" })
            .spawn()
            .unwrap();
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
                    panic!("history fixture failed: {result:?}");
                }
            }
        };
        assert!(
            status.success(),
            "{operation}, absolute={absolute}: {status}"
        );
        assert_eq!(
            std::fs::read(directory.path().join("complete")).unwrap(),
            b"checked"
        );
    }
}

#[test]
fn history_load_rejects_path_escape() {
    run_case("load");
}

#[test]
fn history_save_rejects_path_escape() {
    run_case("save");
}

#[test]
fn history_delete_rejects_path_escape() {
    run_case("delete");
}

#[test]
#[ignore = "invoked by bounded subprocess drivers with isolated configuration roots"]
fn history_path_fixture() {
    let directory = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(claurst_core::Settings::config_dir(), directory);
    if std::env::var(OPERATION).unwrap() == "controls" {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for id in [
                "",
                ".",
                "..",
                "a/b",
                r"a\b",
                "C:relative",
                "name:stream",
                "nul\0id",
            ] {
                let mut session = ConversationSession::new("fixture".into());
                session.id = id.into();
                for error in [
                    save_session(&session).await.unwrap_err(),
                    load_session(id).await.unwrap_err(),
                    delete_session(id).await.unwrap_err(),
                ] {
                    assert!(
                        error.to_string().contains("invalid history session id"),
                        "{error}"
                    );
                }
                assert!(
                    !directory.join("sessions").exists(),
                    "invalid save created sessions directory"
                );
            }
            for id in [
                "safe-id_123.4",
                "custom session",
                "会話-é",
                ".hidden",
                "two..dots",
            ] {
                let mut session = ConversationSession::new("fixture".into());
                session.id = id.into();
                save_session(&session).await.unwrap();
                assert_eq!(load_session(id).await.unwrap().id, id);
                assert!(directory
                    .join("sessions")
                    .join(format!("{id}.json"))
                    .is_file());
                delete_session(id).await.unwrap();
                delete_session(id).await.unwrap();
                assert!(!directory
                    .join("sessions")
                    .join(format!("{id}.json"))
                    .exists());
            }
        });
        std::fs::write(directory.join("complete"), b"checked").unwrap();
        return;
    }
    std::fs::create_dir(directory.join("sessions")).unwrap();
    let outside = directory.join("outside.json");
    let original = serde_json::to_vec(&ConversationSession::new("fixture-model".into())).unwrap();
    std::fs::write(&outside, &original).unwrap();
    let id = if std::env::var(ABSOLUTE).unwrap() == "yes" {
        directory.join("outside").to_str().unwrap().to_owned()
    } else {
        "../outside".into()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let operation = std::env::var(OPERATION).unwrap();
    let rejected = runtime.block_on(async {
        match operation.as_str() {
            "load" => load_session(&id).await.is_err(),
            "save" => {
                let mut session = ConversationSession::new("replacement".into());
                session.id = id;
                save_session(&session).await.is_err()
            }
            "delete" => delete_session(&id).await.is_err(),
            _ => panic!("unknown operation"),
        }
    });
    assert!(
        outside.exists(),
        "{operation} removed a file outside sessions"
    );
    assert!(
        std::fs::read(&outside).unwrap() == original,
        "{operation} modified a file outside sessions"
    );
    assert!(rejected, "{operation} accepted a path-escaping session ID");
    std::fs::write(directory.join("complete"), b"checked").unwrap();
}

#[test]
fn history_path_validation_and_custom_ids() {
    run_case("controls");
}
