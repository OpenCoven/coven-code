//! Read-only snapshot of familiar memory for the Next Memory page (#415).
//!
//! Reads come only from the Coven daemon's `GET /api/v1/memory` and
//! `/overview` routes. There is no filesystem fallback, nothing is sent to a
//! model, and the snapshot lives only while the Memory page is open. Titles
//! and excerpts are already withheld by the adapter
//! ([`claurst_core::coven_daemon::MemoryPreview`]) unless the producer
//! classified the entry and said no reveal is required. Reveal and context
//! selection are not in this slice.

use super::view::terminal_safe;
use claurst_core::coven_daemon::{DaemonClient, DaemonError, MemoryOverview, MemorySummary};

/// What the Memory page knows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MemorySnapshot {
    /// Not read yet in this opening of the page.
    #[default]
    NotRead,
    /// No daemon socket, or it refused the connection.
    Offline,
    /// The daemon answered but has no memory read routes.
    UpdateRequired,
    /// The read failed; one line, no content.
    Failed(String),
    Loaded {
        rows: Vec<MemorySummary>,
        /// The overview, or why it could not be read.
        overview: Result<MemoryOverview, String>,
    },
}

impl MemorySnapshot {
    /// Classify the two reads. A failed list decides the page; a failed
    /// overview is shown next to the rows.
    pub fn from_reads(
        list: Result<Vec<MemorySummary>, DaemonError>,
        overview: Result<MemoryOverview, DaemonError>,
    ) -> Self {
        let reason = |error: DaemonError| one_line(&error.to_string());
        match list {
            Err(error) if error.is_offline() => MemorySnapshot::Offline,
            // The list route exists on every daemon that serves memory, so a
            // 404 means this daemon predates it.
            Err(error) if error.status() == Some(404) => MemorySnapshot::UpdateRequired,
            Err(error) => MemorySnapshot::Failed(reason(error)),
            Ok(rows) => MemorySnapshot::Loaded {
                rows,
                overview: overview.map_err(reason),
            },
        }
    }
}

/// Longest error text kept.
const REASON_MAX_CHARS: usize = 160;

fn one_line(text: &str) -> String {
    let safe = terminal_safe(text).replace('\n', " ");
    if safe.chars().count() <= REASON_MAX_CHARS {
        return safe;
    }
    let mut short: String = safe.chars().take(REASON_MAX_CHARS).collect();
    short.push('…');
    short
}

/// Blocking: read the daemon once. Call off the UI thread.
pub fn read_memory(client: Option<DaemonClient>) -> MemorySnapshot {
    let Some(client) = client else {
        return MemorySnapshot::Offline;
    };
    MemorySnapshot::from_reads(client.memory_list(), client.memory_overview())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> MemorySummary {
        MemorySummary {
            id: "d251bc66-3e45-5d03-8d78-1e76919642f9".into(),
            familiar_id: "sage".into(),
            updated_at: None,
            source_label: None,
            verification_state: "unknown".into(),
            preview: claurst_core::coven_daemon::MemoryPreview::Withheld,
        }
    }

    #[test]
    fn reads_are_classified_without_inventing_data() {
        let offline = DaemonError::Offline {
            reason: "no socket".into(),
        };
        assert_eq!(
            MemorySnapshot::from_reads(Err(offline.clone()), Err(offline)),
            MemorySnapshot::Offline
        );
        let missing = || DaemonError::BadStatus {
            status: 404,
            code: Some("not_found".into()),
            message: None,
        };
        assert_eq!(
            MemorySnapshot::from_reads(Err(missing()), Err(missing())),
            MemorySnapshot::UpdateRequired
        );
        let broken = DaemonError::BadStatus {
            status: 503,
            code: Some("memory_content_unavailable".into()),
            message: Some("line one\nline two".into()),
        };
        let MemorySnapshot::Failed(reason) =
            MemorySnapshot::from_reads(Err(broken), Ok(MemoryOverview::default()))
        else {
            panic!("a failed list is a failure, not an empty memory");
        };
        assert!(reason.contains("503") && !reason.contains('\n'), "{reason}");

        let loaded = MemorySnapshot::from_reads(Ok(vec![row()]), Err(missing()));
        let MemorySnapshot::Loaded { rows, overview } = loaded else {
            panic!()
        };
        assert_eq!(rows.len(), 1);
        assert!(overview.is_err());
    }

    #[test]
    fn no_daemon_is_offline_and_reads_no_file() {
        assert_eq!(read_memory(None), MemorySnapshot::Offline);
    }
}
