// sqlite_storage.rs — Optional SQLite-backed session storage.
//
// Provides `SqliteSessionStore` as a faster, queryable alternative to
// the default JSONL storage.  Enabled by adding `rusqlite` to the
// crate's dependencies (already done via `features = ["bundled"]`).

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use crate::crypto_utils::sha256_hex_str;
use crate::types::{ContentBlock, Message, MessageContent, Role};

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

    /// Index a session's messages for search, one row per message.
    ///
    /// A message with a uuid is stored under it. A message without one (most
    /// user messages, tool results included) is stored under an id derived
    /// from the session, its role, the SHA-256 of its text and how many earlier
    /// uuid-less messages in the session have the same role and text. Saving
    /// the same session again, or after it has grown, derives the same ids for
    /// the messages already indexed, so it adds rows only for new ones; the
    /// same text sent twice is two rows.
    ///
    /// The id follows the text, not the position. After a compaction or a
    /// rewind, a message whose role and text match one already indexed maps
    /// onto that row and adds nothing; search still finds the session. A
    /// message with no text, such as a tool result, is indexed with empty
    /// content, as an assistant message holding only a tool call already is.
    ///
    /// Indexes written before #566 keep their one row with the id "unknown"
    /// (the first uuid-less message ever indexed). It is not deleted, so its
    /// session can hold that text twice once it is saved again.
    ///
    /// Best effort per message: every message is tried, and the first error is
    /// returned.
    pub fn index_messages<'a>(
        &self,
        session_id: &str,
        messages: impl IntoIterator<Item = &'a Message>,
    ) -> anyhow::Result<()> {
        let mut earlier: HashMap<(&str, String), usize> = HashMap::new();
        let mut first_error = None;
        for message in messages {
            let role = match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            let content: Cow<'_, str> = match &message.content {
                MessageContent::Text(text) => Cow::Borrowed(text),
                MessageContent::Blocks(blocks) => Cow::Owned(
                    blocks
                        .iter()
                        .filter_map(|block| match block {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
            };
            let id = match message.uuid.as_deref() {
                Some(uuid) => Cow::Borrowed(uuid),
                None => {
                    let hash = sha256_hex_str(&content);
                    let occurrence = earlier.entry((role, hash.clone())).or_insert(0);
                    let id = format!("{session_id}:{role}:{hash}:{occurrence}");
                    *occurrence += 1;
                    Cow::Owned(id)
                }
            };
            if let Err(error) = self.save_message(session_id, &id, role, &content, None) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
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

        let rows = stmt.query_map([], session_summary_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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

        let rows = stmt.query_map(rusqlite::params![like], session_summary_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Delete a session and all of its messages atomically.
    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        transaction.execute(
            "DELETE FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
        )?;
        transaction.commit()?;
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

fn session_summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        model: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        message_count: row.get::<_, Option<u32>>(5)?.unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolResultContent;

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

    fn stored_rows(store: &SqliteSessionStore) -> Vec<(String, String)> {
        let mut statement = store
            .conn
            .prepare("SELECT id, content FROM messages ORDER BY content, id")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn index_messages_keeps_each_message_once_across_saves() {
        let (_dir, store) = store_with_session();
        let mut answer = Message::assistant_blocks(vec![
            ContentBlock::Text {
                text: "answer".into(),
            },
            ContentBlock::Text {
                text: "text".into(),
            },
        ]);
        answer.uuid = Some("answer-uuid".into());
        let tool_result = || {
            Message::user_blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tool-1".into(),
                content: ToolResultContent::Text("output".into()),
                is_error: None,
            }])
        };
        let messages = vec![
            Message::user("hello"),
            answer,
            tool_result(),
            Message::user("again"),
            tool_result(),
            Message::user("hello"),
        ];
        // Each save holds the whole transcript so far, which grows, and the
        // last one is saved twice.
        for end in [1, 3, messages.len(), messages.len()] {
            store.index_messages("session-1", &messages[..end]).unwrap();
        }

        let rows = stored_rows(&store);
        let contents: Vec<_> = rows.iter().map(|(_, content)| content.as_str()).collect();
        assert_eq!(contents, ["", "", "again", "answer text", "hello", "hello"]);
        assert!(rows.iter().any(|(id, _)| id == "answer-uuid"));
        assert!(rows.iter().all(|(id, _)| id != "unknown"));
        assert_eq!(session_state(&store).1, 6);
        for text in ["hello", "again", "answer"] {
            assert_eq!(store.search_sessions(text).unwrap().len(), 1, "{text}");
        }
    }
}
