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
        match list {
            Err(error) if error.is_offline() => MemorySnapshot::Offline,
            // A 404 on the list route, or an older row shape, means this
            // daemon predates the v1 memory reads.
            Err(error)
                if error.status() == Some(404)
                    || matches!(error, DaemonError::UnsupportedShape(_)) =>
            {
                MemorySnapshot::UpdateRequired
            }
            Err(error) => MemorySnapshot::Failed(reason(&error)),
            Ok(rows) => MemorySnapshot::Loaded {
                rows,
                overview: overview.map_err(|error| reason(&error)),
            },
        }
    }
}

/// Why a read failed, without any producer text: a daemon message, a parse
/// error or a transport error can quote memory content or paths.
fn reason(error: &DaemonError) -> String {
    match error {
        DaemonError::BadStatus { status, code, .. } => match code {
            Some(code) => format!("HTTP {status} ({})", one_line(code)),
            None => format!("HTTP {status}"),
        },
        DaemonError::Offline { .. } => "daemon offline".to_string(),
        DaemonError::Transport(_) => "daemon transport error".to_string(),
        DaemonError::MalformedResponse(_) => "unreadable daemon response".to_string(),
        DaemonError::MissingField(field) => format!("daemon response missing {field}"),
        DaemonError::UnsupportedShape(what) => format!("older daemon response ({what})"),
    }
}

/// Longest error code kept.
const REASON_MAX_CHARS: usize = 64;

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
    let list = client.memory_list();
    // The overview is only worth a second round trip when the list read.
    let overview = match &list {
        Ok(_) => client.memory_overview(),
        Err(_) => Err(DaemonError::MissingField("overview")),
    };
    MemorySnapshot::from_reads(list, overview)
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
    fn a_failed_read_shows_no_producer_text() {
        const CANARY: &str = "CANARY-7f3a-private";
        for error in [
            DaemonError::BadStatus {
                status: 500,
                code: Some("internal".into()),
                message: Some(format!("failed to read sage/{CANARY}.md")),
            },
            DaemonError::MalformedResponse(format!("invalid type: string \"{CANARY}\"")),
            DaemonError::Transport(format!("read {CANARY}")),
        ] {
            let MemorySnapshot::Failed(reason) =
                MemorySnapshot::from_reads(Err(error), Ok(MemoryOverview::default()))
            else {
                panic!()
            };
            assert!(!reason.contains(CANARY), "{reason}");
        }
        let loaded = MemorySnapshot::from_reads(
            Ok(Vec::new()),
            Err(DaemonError::MalformedResponse(CANARY.into())),
        );
        let MemorySnapshot::Loaded { overview, .. } = loaded else {
            panic!()
        };
        assert!(!overview.unwrap_err().contains(CANARY));
    }

    #[test]
    fn an_unsupported_response_shape_means_update_required() {
        assert_eq!(
            MemorySnapshot::from_reads(
                Err(DaemonError::UnsupportedShape("memory row id")),
                Err(DaemonError::UnsupportedShape("memory row id")),
            ),
            MemorySnapshot::UpdateRequired
        );
    }

    #[test]
    fn no_daemon_is_offline_and_reads_no_file() {
        assert_eq!(read_memory(None), MemorySnapshot::Offline);
    }
}
