// sqlite_storage.rs — Optional SQLite-backed session storage.
//
// Provides `SqliteSessionStore` as a faster, queryable alternative to
// the default JSONL storage.  Enabled by adding `rusqlite` to the
// crate's dependencies (already done via `features = ["bundled"]`).

use std::path::Path;

/// A persistent SQLite session + message store.
pub struct SqliteSessionStore {
    conn: rusqlite::Connection,
}

impl SqliteSessionStore {
    /// Open (or create) the database at `db_path` and ensure the schema exists.
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY,
                title       TEXT,
                model       TEXT NOT NULL DEFAULT '',
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                message_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS messages (
                id          TEXT PRIMARY KEY,
                session_id  TEXT NOT NULL REFERENCES sessions(id),
                role        TEXT NOT NULL,
                content     TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                cost_usd    REAL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_session
                ON messages(session_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_updated
                ON sessions(updated_at);
            ",
        )?;

        Ok(Self { conn })
    }

    /// Insert or replace a session record.  `created_at` is preserved on
    /// UPDATE so only `updated_at` changes.
    pub fn save_session(
        &self,
        session_id: &str,
        title: Option<&str>,
        model: &str,
    ) -> anyhow::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO sessions (id, title, model, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 title      = excluded.title,
                 model      = excluded.model,
                 updated_at = excluded.updated_at",
            rusqlite::params![session_id, title, model, now],
        )?;
        Ok(())
    }

    /// Append a message to the given session (idempotent on `msg_id`).
    /// Also bumps `sessions.message_count` and `sessions.updated_at`.
    pub fn save_message(
        &self,
        session_id: &str,
        msg_id: &str,
        role: &str,
        content: &str,
        cost_usd: Option<f64>,
    ) -> anyhow::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let transaction = self.conn.unchecked_transaction()?;
        // Insert the message; ignore if already stored.
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO messages
             (id, session_id, role, content, created_at, cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![msg_id, session_id, role, content, now, cost_usd],
        )?;
        // Only bump count when we actually inserted a new row.
        if inserted > 0 {
            transaction.execute(
                "UPDATE sessions
                 SET updated_at    = ?1,
                     message_count = message_count + 1
                 WHERE id = ?2",
                rusqlite::params![now, session_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Return the 100 most recently updated sessions.
    pub fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, model, created_at, updated_at, message_count
             FROM sessions
             ORDER BY updated_at DESC
             LIMIT 100",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                model: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                message_count: row.get::<_, Option<u32>>(5)?.unwrap_or(0),
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Full-text search across session titles and message content.
    /// Returns up to 50 matching sessions ordered by recency.
    pub fn search_sessions(&self, query: &str) -> anyhow::Result<Vec<SessionSummary>> {
        let like = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT s.id, s.title, s.model,
                    s.created_at, s.updated_at, s.message_count
             FROM sessions s
             LEFT JOIN messages m ON m.session_id = s.id
             WHERE s.title LIKE ?1
                OR m.content LIKE ?1
             ORDER BY s.updated_at DESC
             LIMIT 50",
        )?;

        let rows = stmt.query_map(rusqlite::params![like], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                model: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                message_count: row.get::<_, Option<u32>>(5)?.unwrap_or(0),
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Delete a session and all of its messages.
    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        self.conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
        )?;
        Ok(())
    }
}

/// Summary row returned by `list_sessions` and `search_sessions`.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub title: Option<String>,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_session() -> (tempfile::TempDir, SqliteSessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteSessionStore::open(&dir.path().join("sessions.db")).unwrap();
        store
            .save_session("session-1", Some("Test session"), "test-model")
            .unwrap();
        (dir, store)
    }

    fn stored_message_count(store: &SqliteSessionStore, message_id: &str) -> i64 {
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE id = ?1",
                rusqlite::params![message_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn session_state(store: &SqliteSessionStore) -> (String, i64) {
        store
            .conn
            .query_row(
                "SELECT updated_at, message_count FROM sessions WHERE id = 'session-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn save_message_rolls_back_insert_when_session_update_fails() {
        let (_dir, store) = store_with_session();
        let initial_session_state = session_state(&store);
        store
            .conn
            .execute_batch(
                "
                CREATE TEMP TRIGGER fail_session_update
                BEFORE UPDATE OF updated_at, message_count ON sessions
                BEGIN
                    SELECT RAISE(ABORT, 'injected session update failure');
                END;
                ",
            )
            .unwrap();

        let error = store
            .save_message("session-1", "message-1", "user", "first attempt", None)
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("injected session update failure"));
        assert_eq!(stored_message_count(&store, "message-1"), 0);
        assert_eq!(session_state(&store), initial_session_state);

        store
            .conn
            .execute_batch("DROP TRIGGER fail_session_update;")
            .unwrap();

        store
            .save_message("session-1", "message-1", "user", "retry", None)
            .unwrap();
        store
            .save_message("session-1", "message-1", "user", "duplicate retry", None)
            .unwrap();

        assert_eq!(stored_message_count(&store, "message-1"), 1);
        assert_eq!(session_state(&store).1, 1);
    }

    #[test]
    fn save_message_is_idempotent_for_duplicate_message_ids() {
        let (_dir, store) = store_with_session();

        store
            .save_message(
                "session-1",
                "message-1",
                "user",
                "original content",
                Some(0.25),
            )
            .unwrap();
        let session_state_after_insert = session_state(&store);

        store
            .save_message(
                "session-1",
                "message-1",
                "assistant",
                "replacement content",
                Some(1.0),
            )
            .unwrap();

        let stored_message = store
            .conn
            .query_row(
                "SELECT role, content, cost_usd FROM messages WHERE id = 'message-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<f64>>(2)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(stored_message_count(&store, "message-1"), 1);
        assert_eq!(session_state(&store), session_state_after_insert);
        assert_eq!(
            stored_message,
            ("user".to_owned(), "original content".to_owned(), Some(0.25))
        );
    }
}
