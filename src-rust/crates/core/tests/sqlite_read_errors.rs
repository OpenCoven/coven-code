use claurst_core::sqlite_storage::SqliteSessionStore;
use rusqlite::{types::Type, Connection, Error};

// SQLite quote() retains NULL, storage type and exact fixture values, including BLOBs.
fn snapshot(conn: &Connection) -> (Vec<[String; 6]>, Vec<[String; 6]>) {
    let read = |sql: &str| {
        conn.prepare(sql)
            .unwrap()
            .query_map([], |row| {
                Ok([
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ])
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    (
        read("SELECT quote(id), quote(title), quote(model), quote(created_at), quote(updated_at), quote(message_count) FROM sessions ORDER BY id"),
        read("SELECT quote(id), quote(session_id), quote(role), quote(content), quote(created_at), quote(cost_usd) FROM messages ORDER BY id"),
    )
}

fn corrupt_row_is_error(search: bool, blob: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let store = SqliteSessionStore::open(&path).unwrap();
    store
        .save_session("valid", Some("matching valid"), "model")
        .unwrap();
    store
        .save_session("corrupt", Some("matching corrupt"), "model")
        .unwrap();
    store
        .save_message("valid", "message", "user", "matching content", Some(0.5))
        .unwrap();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(if blob {
        "UPDATE sessions SET model = X'0102' WHERE id = 'corrupt'"
    } else {
        "UPDATE sessions SET message_count = -1 WHERE id = 'corrupt'"
    })
    .unwrap();
    let before = snapshot(&conn);
    assert_eq!(before.0.len(), 2);
    assert_eq!(before.1.len(), 1);
    let result = if search {
        store.search_sessions("matching")
    } else {
        store.list_sessions()
    };
    assert_eq!(snapshot(&conn), before);
    let error = result.unwrap_err();
    match error.downcast_ref::<Error>() {
        Some(Error::InvalidColumnType(2, name, Type::Blob)) if blob => assert_eq!(name, "model"),
        Some(Error::IntegralValueOutOfRange(5, -1)) if !blob => {}
        other => panic!("unexpected row error: {other:?}"),
    }
}

#[test]
fn list_rejects_negative_count() {
    corrupt_row_is_error(false, false);
}
#[test]
fn search_rejects_negative_count() {
    corrupt_row_is_error(true, false);
}
#[test]
fn list_rejects_blob_model() {
    corrupt_row_is_error(false, true);
}
#[test]
fn search_rejects_blob_model() {
    corrupt_row_is_error(true, true);
}

#[test]
fn valid_order_and_search_selection_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let store = SqliteSessionStore::open(&path).unwrap();
    assert!(store.list_sessions().unwrap().is_empty());
    assert!(store.search_sessions("needle").unwrap().is_empty());
    store
        .save_session("older", Some("needle title"), "old-model")
        .unwrap();
    store.save_session("newer", None, "new-model").unwrap();
    store
        .save_session("excluded", Some("unrelated"), "other-model")
        .unwrap();
    store
        .save_message("newer", "message", "user", "needle body", None)
        .unwrap();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("UPDATE sessions SET updated_at = CASE id WHEN 'older' THEN '2024-01-01' WHEN 'newer' THEN '2024-01-03' ELSE '2024-01-02' END").unwrap();
    let before = snapshot(&conn);
    let all = store.list_sessions().unwrap();
    assert_eq!(
        all.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["newer", "excluded", "older"]
    );
    let matches = store.search_sessions("needle").unwrap();
    assert_eq!(
        matches
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["newer", "older"]
    );
    assert_eq!(matches[0].title, None);
    assert_eq!(matches[0].model, "new-model");
    assert_eq!(matches[0].message_count, 1);
    assert_eq!(matches[0].updated_at, "2024-01-03");
    assert!(!matches[0].created_at.is_empty());
    assert_eq!(snapshot(&conn), before);
    conn.execute(
        "UPDATE sessions SET message_count=-1 WHERE id='excluded'",
        [],
    )
    .unwrap();
    let corrupt_snapshot = snapshot(&conn);
    assert_eq!(store.search_sessions("needle").unwrap().len(), 2);
    assert_eq!(snapshot(&conn), corrupt_snapshot);
}

#[test]
fn compatible_nullable_schema_keeps_existing_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, model TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, message_count INTEGER); INSERT INTO sessions VALUES ('nullable', 'needle', NULL, 'created', 'updated', NULL)").unwrap();
    let store = SqliteSessionStore::open(&path).unwrap();
    let before = snapshot(&conn);
    for rows in [
        store.list_sessions().unwrap(),
        store.search_sessions("needle").unwrap(),
    ] {
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.id, "nullable");
        assert_eq!(row.title.as_deref(), Some("needle"));
        assert_eq!(row.model, "");
        assert_eq!(row.created_at, "created");
        assert_eq!(row.updated_at, "updated");
        assert_eq!(row.message_count, 0);
    }
    assert_eq!(snapshot(&conn), before);
}
