use claurst_core::sqlite_storage::SqliteSessionStore;
use rusqlite::Connection;

#[derive(Debug, PartialEq)]
struct SessionRow {
    id: String,
    title: Option<String>,
    model: String,
    created_at: String,
    updated_at: String,
    message_count: i64,
}

#[derive(Debug, PartialEq)]
struct MessageRow {
    id: String,
    session_id: String,
    role: String,
    content: String,
    created_at: String,
    cost_usd: Option<f64>,
}

fn snapshot(conn: &Connection) -> (Vec<SessionRow>, Vec<MessageRow>) {
    let sessions = conn
        .prepare("SELECT id, title, model, created_at, updated_at, message_count FROM sessions ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok(SessionRow {
            id: row.get(0)?, title: row.get(1)?, model: row.get(2)?,
            created_at: row.get(3)?, updated_at: row.get(4)?, message_count: row.get(5)?,
        }))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let messages = conn
        .prepare(
            "SELECT id, session_id, role, content, created_at, cost_usd FROM messages ORDER BY id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok(MessageRow {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                cost_usd: row.get(5)?,
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    (sessions, messages)
}

fn check_failed_delete(trigger: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let store = SqliteSessionStore::open(&path).unwrap();
    store
        .save_session("target", Some("Target title"), "target-model")
        .unwrap();
    store
        .save_session("unrelated", None, "other-model")
        .unwrap();
    store
        .save_message("target", "target-user", "user", "Original question", None)
        .unwrap();
    store
        .save_message(
            "target",
            "target-assistant",
            "assistant",
            "Original answer",
            Some(0.25),
        )
        .unwrap();
    store
        .save_message(
            "unrelated",
            "other-message",
            "user",
            "Other content",
            Some(0.5),
        )
        .unwrap();
    let conn = Connection::open(&path).unwrap();
    let before = snapshot(&conn);
    assert_eq!(before.0.len(), 2);
    assert_eq!(before.1.len(), 3);
    conn.execute_batch(trigger).unwrap();

    let error = store.delete_session("target").unwrap_err();
    match error.downcast_ref::<rusqlite::Error>() {
        Some(rusqlite::Error::SqliteFailure(_, Some(message))) => {
            assert_eq!(message, "fixture delete blocked");
        }
        other => panic!("unexpected delete error: {other:?}"),
    }
    assert_eq!(
        snapshot(&conn),
        before,
        "failed deletion must preserve every stored field"
    );

    conn.execute_batch("DROP TRIGGER block_delete").unwrap();
    store.delete_session("target").unwrap();
    let expected = (
        before
            .0
            .into_iter()
            .filter(|row| row.id == "unrelated")
            .collect(),
        before
            .1
            .into_iter()
            .filter(|row| row.session_id == "unrelated")
            .collect(),
    );
    assert_eq!(snapshot(&conn), expected);
    store.delete_session("target").unwrap();
    assert_eq!(
        snapshot(&conn),
        expected,
        "absent deletion must be idempotent"
    );
}

#[test]
fn session_delete_failure_rolls_back_message_deletion() {
    check_failed_delete("CREATE TRIGGER block_delete BEFORE DELETE ON sessions WHEN OLD.id = 'target' BEGIN SELECT RAISE(ABORT, 'fixture delete blocked'); END;");
}

#[test]
fn message_delete_failure_preserves_session_and_messages() {
    check_failed_delete("CREATE TRIGGER block_delete BEFORE DELETE ON messages WHEN OLD.session_id = 'target' BEGIN SELECT RAISE(ABORT, 'fixture delete blocked'); END;");
}
