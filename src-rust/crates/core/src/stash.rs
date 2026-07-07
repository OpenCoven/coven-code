//! Stash — durable, structured storage for links and attachments.
//!
//! Backs the `/link` and `/attach` slash commands. Items are stored in
//! `~/.coven-code/stash.sqlite`; attachment files are copied into
//! `~/.coven-code/attachments/<id>/<filename>` so they survive the original
//! file moving or the session ending.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StashKind {
    Link,
    Attachment,
}

impl StashKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            StashKind::Link => "link",
            StashKind::Attachment => "attachment",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "link" => Some(StashKind::Link),
            "attachment" => Some(StashKind::Attachment),
            _ => None,
        }
    }
}

/// A single stashed link or attachment.
#[derive(Debug, Clone)]
pub struct StashItem {
    pub id: String,
    pub kind: StashKind,
    /// The URL for links; the stored (copied) file path for attachments.
    pub value: String,
    /// Original source path for attachments (informational).
    pub original_path: Option<String>,
    pub title: Option<String>,
    pub note: Option<String>,
    pub tags: Vec<String>,
    /// Project root the item was saved from.
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub created_at_ms: u64,
}

impl StashItem {
    /// Short display id (first 8 chars of the uuid).
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(8)]
    }
}

/// Filters for listing stash items.
#[derive(Debug, Clone, Default)]
pub struct StashFilter {
    pub kind: Option<StashKind>,
    pub tag: Option<String>,
    pub project: Option<String>,
}

#[derive(Debug)]
pub enum StashError {
    Db(String),
    Io(String),
    NotFound(String),
    Ambiguous(String),
}

impl std::fmt::Display for StashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StashError::Db(msg) => write!(f, "stash database error: {}", msg),
            StashError::Io(msg) => write!(f, "stash I/O error: {}", msg),
            StashError::NotFound(id) => write!(f, "no stash item matches '{}'", id),
            StashError::Ambiguous(id) => {
                write!(
                    f,
                    "'{}' matches more than one stash item; use a longer id",
                    id
                )
            }
        }
    }
}

impl std::error::Error for StashError {}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

pub struct StashStore {
    conn: rusqlite::Connection,
}

impl StashStore {
    /// Open (or create) a stash database.
    pub fn open(db_path: &Path) -> Result<Self, StashError> {
        let conn =
            rusqlite::Connection::open(db_path).map_err(|e| StashError::Db(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS stash_items (
                id             TEXT PRIMARY KEY,
                kind           TEXT NOT NULL,
                value          TEXT NOT NULL,
                original_path  TEXT,
                title          TEXT,
                note           TEXT,
                tags           TEXT NOT NULL DEFAULT '',
                project        TEXT,
                session_id     TEXT,
                created_at_ms  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_stash_kind ON stash_items(kind);
            CREATE INDEX IF NOT EXISTS idx_stash_project ON stash_items(project);",
        )
        .map_err(|e| StashError::Db(e.to_string()))?;
        Ok(Self { conn })
    }

    /// Default database path: `~/.coven-code/stash.sqlite`.
    pub fn default_path() -> PathBuf {
        crate::config::Settings::config_dir().join("stash.sqlite")
    }

    /// Default directory for stored attachment copies.
    pub fn default_attachments_dir() -> PathBuf {
        crate::config::Settings::config_dir().join("attachments")
    }

    /// Open using the default path (creates `~/.coven-code` if needed).
    pub fn open_default() -> Result<Self, StashError> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StashError::Io(e.to_string()))?;
        }
        Self::open(&path)
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Save a link.
    pub fn add_link(
        &self,
        url: &str,
        title: Option<&str>,
        note: Option<&str>,
        tags: &[String],
        project: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<StashItem, StashError> {
        let item = StashItem {
            id: uuid::Uuid::new_v4().simple().to_string(),
            kind: StashKind::Link,
            value: url.to_string(),
            original_path: None,
            title: title.map(str::to_string),
            note: note.map(str::to_string),
            tags: tags.to_vec(),
            project: project.map(str::to_string),
            session_id: session_id.map(str::to_string),
            created_at_ms: Self::now_ms(),
        };
        self.insert(&item)?;
        Ok(item)
    }

    /// Save an attachment: copies `src` into `attachments_dir/<id>/<filename>`
    /// and records the stored copy.
    // Metadata fields (title/note/tags/project/session) are individually
    // optional; a params struct would just restate the StashItem shape.
    #[allow(clippy::too_many_arguments)]
    pub fn add_attachment(
        &self,
        src: &Path,
        attachments_dir: &Path,
        title: Option<&str>,
        note: Option<&str>,
        tags: &[String],
        project: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<StashItem, StashError> {
        if !src.is_file() {
            return Err(StashError::Io(format!(
                "'{}' is not a readable file",
                src.display()
            )));
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let filename = src
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "attachment".to_string());
        let dest_dir = attachments_dir.join(&id[..8]);
        std::fs::create_dir_all(&dest_dir).map_err(|e| StashError::Io(e.to_string()))?;
        let dest = dest_dir.join(&filename);
        std::fs::copy(src, &dest).map_err(|e| StashError::Io(e.to_string()))?;

        let item = StashItem {
            id,
            kind: StashKind::Attachment,
            value: dest.to_string_lossy().to_string(),
            original_path: Some(src.to_string_lossy().to_string()),
            title: title.map(str::to_string),
            note: note.map(str::to_string),
            tags: tags.to_vec(),
            project: project.map(str::to_string),
            session_id: session_id.map(str::to_string),
            created_at_ms: Self::now_ms(),
        };
        self.insert(&item)?;
        Ok(item)
    }

    fn insert(&self, item: &StashItem) -> Result<(), StashError> {
        self.conn
            .execute(
                "INSERT INTO stash_items
                 (id, kind, value, original_path, title, note, tags, project, session_id, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    item.id,
                    item.kind.as_str(),
                    item.value,
                    item.original_path,
                    item.title,
                    item.note,
                    item.tags.join(","),
                    item.project,
                    item.session_id,
                    item.created_at_ms,
                ],
            )
            .map_err(|e| StashError::Db(e.to_string()))?;
        Ok(())
    }

    /// List items matching the filter, newest first.
    pub fn list(&self, filter: &StashFilter) -> Result<Vec<StashItem>, StashError> {
        let mut items = self.query_all()?;
        items.retain(|item| {
            if let Some(kind) = filter.kind {
                if item.kind != kind {
                    return false;
                }
            }
            if let Some(ref tag) = filter.tag {
                if !item.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                    return false;
                }
            }
            if let Some(ref project) = filter.project {
                if item.project.as_deref() != Some(project.as_str()) {
                    return false;
                }
            }
            true
        });
        Ok(items)
    }

    /// Case-insensitive substring search over value, title, note, and tags.
    pub fn search(
        &self,
        term: &str,
        kind: Option<StashKind>,
    ) -> Result<Vec<StashItem>, StashError> {
        let needle = term.to_lowercase();
        let mut items = self.query_all()?;
        items.retain(|item| {
            if let Some(k) = kind {
                if item.kind != k {
                    return false;
                }
            }
            item.value.to_lowercase().contains(&needle)
                || item
                    .title
                    .as_deref()
                    .is_some_and(|t| t.to_lowercase().contains(&needle))
                || item
                    .note
                    .as_deref()
                    .is_some_and(|n| n.to_lowercase().contains(&needle))
                || item.tags.iter().any(|t| t.to_lowercase().contains(&needle))
        });
        Ok(items)
    }

    /// Find a single item by full id or unique id prefix.
    pub fn get(&self, id_prefix: &str) -> Result<StashItem, StashError> {
        let mut matches = self
            .query_all()?
            .into_iter()
            .filter(|item| item.id.starts_with(id_prefix));
        match (matches.next(), matches.next()) {
            (None, _) => Err(StashError::NotFound(id_prefix.to_string())),
            (Some(item), None) => Ok(item),
            (Some(_), Some(_)) => Err(StashError::Ambiguous(id_prefix.to_string())),
        }
    }

    /// Remove an item by id prefix. For attachments the stored copy (and its
    /// per-item directory) is deleted as well.
    pub fn remove(&self, id_prefix: &str) -> Result<StashItem, StashError> {
        let item = self.get(id_prefix)?;
        self.conn
            .execute("DELETE FROM stash_items WHERE id = ?1", [&item.id])
            .map_err(|e| StashError::Db(e.to_string()))?;
        if item.kind == StashKind::Attachment {
            let stored = Path::new(&item.value);
            let _ = std::fs::remove_file(stored);
            if let Some(dir) = stored.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        }
        Ok(item)
    }

    fn query_all(&self) -> Result<Vec<StashItem>, StashError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, value, original_path, title, note, tags, project,
                        session_id, created_at_ms
                 FROM stash_items ORDER BY created_at_ms DESC",
            )
            .map_err(|e| StashError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                let kind_str: String = row.get(1)?;
                let tags_str: String = row.get(6)?;
                Ok(StashItem {
                    id: row.get(0)?,
                    kind: StashKind::parse(&kind_str).unwrap_or(StashKind::Link),
                    value: row.get(2)?,
                    original_path: row.get(3)?,
                    title: row.get(4)?,
                    note: row.get(5)?,
                    tags: tags_str
                        .split(',')
                        .filter(|t| !t.is_empty())
                        .map(str::to_string)
                        .collect(),
                    project: row.get(7)?,
                    session_id: row.get(8)?,
                    created_at_ms: row.get(9)?,
                })
            })
            .map_err(|e| StashError::Db(e.to_string()))?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(|e| StashError::Db(e.to_string()))?);
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, StashStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = StashStore::open(&dir.path().join("stash.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn add_and_list_links() {
        let (_dir, store) = temp_store();
        store
            .add_link(
                "https://example.com/a",
                Some("Example A"),
                None,
                &["docs".to_string()],
                Some("/repo"),
                Some("sess-1"),
            )
            .unwrap();
        store
            .add_link("https://example.com/b", None, None, &[], None, None)
            .unwrap();

        let all = store.list(&StashFilter::default()).unwrap();
        assert_eq!(all.len(), 2);

        let tagged = store
            .list(&StashFilter {
                tag: Some("docs".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].value, "https://example.com/a");
        assert_eq!(tagged[0].title.as_deref(), Some("Example A"));
    }

    #[test]
    fn add_attachment_copies_file_and_remove_cleans_up() {
        let (dir, store) = temp_store();
        let src = dir.path().join("notes.txt");
        std::fs::write(&src, "hello").unwrap();
        let attachments = dir.path().join("attachments");

        let item = store
            .add_attachment(&src, &attachments, None, None, &[], None, None)
            .unwrap();
        let stored = PathBuf::from(&item.value);
        assert!(stored.exists());
        assert_eq!(std::fs::read_to_string(&stored).unwrap(), "hello");
        assert_eq!(item.original_path.as_deref(), Some(src.to_str().unwrap()));

        // Source can vanish; the stored copy remains.
        std::fs::remove_file(&src).unwrap();
        assert!(stored.exists());

        let removed = store.remove(item.short_id()).unwrap();
        assert_eq!(removed.id, item.id);
        assert!(!stored.exists());
        assert!(store.list(&StashFilter::default()).unwrap().is_empty());
    }

    #[test]
    fn attachment_of_missing_file_errors() {
        let (dir, store) = temp_store();
        let err = store
            .add_attachment(
                &dir.path().join("nope.bin"),
                &dir.path().join("attachments"),
                None,
                None,
                &[],
                None,
                None,
            )
            .unwrap_err();
        assert!(matches!(err, StashError::Io(_)));
    }

    #[test]
    fn search_matches_title_note_tags_and_value() {
        let (_dir, store) = temp_store();
        store
            .add_link(
                "https://docs.rs/rusqlite",
                Some("Rusqlite docs"),
                Some("sqlite bindings"),
                &["rust".to_string()],
                None,
                None,
            )
            .unwrap();
        store
            .add_link("https://example.com", None, None, &[], None, None)
            .unwrap();

        assert_eq!(store.search("rusqlite", None).unwrap().len(), 1);
        assert_eq!(store.search("BINDINGS", None).unwrap().len(), 1);
        assert_eq!(store.search("rust", None).unwrap().len(), 1);
        assert_eq!(store.search("zzz", None).unwrap().len(), 0);
    }

    #[test]
    fn get_by_prefix_and_ambiguity() {
        let (_dir, store) = temp_store();
        let a = store
            .add_link("https://a.example", None, None, &[], None, None)
            .unwrap();
        store
            .add_link("https://b.example", None, None, &[], None, None)
            .unwrap();

        assert_eq!(store.get(&a.id[..8]).unwrap().id, a.id);
        assert!(matches!(store.get("zzzzzz"), Err(StashError::NotFound(_))));
        // Empty prefix matches everything → ambiguous.
        assert!(matches!(store.get(""), Err(StashError::Ambiguous(_))));
    }
}
