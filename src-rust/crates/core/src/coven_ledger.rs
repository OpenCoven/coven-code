//! Best-effort registration of interactive sessions in the Coven daemon ledger.
//!
//! Every failure is swallowed to a debug log — a dead or absent daemon must
//! never affect the TUI.

const COVEN_SESSION_SOURCE: &str = "COVEN_SESSION_SOURCE";
const PSYCHE_SESSION_SOURCE: &str = "psyche-build";

fn registration_labels(source: Option<&str>) -> Vec<String> {
    match source {
        Some(PSYCHE_SESSION_SOURCE) => vec![format!("source:{PSYCHE_SESSION_SOURCE}")],
        _ => Vec::new(),
    }
}

#[cfg(unix)]
pub fn notify_session_start(id: &str, project_root: &std::path::Path, title: &str) {
    let Some(client) = crate::coven_daemon::DaemonClient::new() else {
        return;
    };
    let transcript_path = crate::session_storage::transcript_path(project_root, id)
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let req = crate::coven_daemon::RegisterExternalSession {
        id: id.to_string(),
        project_root: project_root.to_string_lossy().into_owned(),
        harness: "coven-code".to_string(),
        title: title.to_string(),
        transcript_path,
        labels: registration_labels(std::env::var(COVEN_SESSION_SOURCE).ok().as_deref()),
    };
    if let Err(e) = client.register_external_session(&req) {
        tracing::debug!("coven ledger register failed (ignored): {e}");
    }
}

#[cfg(unix)]
pub fn notify_session_complete(id: &str, exit_code: Option<i32>) {
    let Some(client) = crate::coven_daemon::DaemonClient::new() else {
        return;
    };
    if let Err(e) = client.complete_session(id, exit_code) {
        tracing::debug!("coven ledger complete failed (ignored): {e}");
    }
}

#[cfg(not(unix))]
pub fn notify_session_start(_id: &str, _project_root: &std::path::Path, _title: &str) {}

#[cfg(not(unix))]
pub fn notify_session_complete(_id: &str, _exit_code: Option<i32>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_only_the_exact_psyche_source() {
        assert_eq!(
            registration_labels(Some(PSYCHE_SESSION_SOURCE)),
            vec![format!("source:{PSYCHE_SESSION_SOURCE}")]
        );

        for source in [
            None,
            Some(""),
            Some("Psyche-Build"),
            Some("foreign"),
            Some("psyche-build-extra"),
        ] {
            assert!(
                registration_labels(source).is_empty(),
                "unexpected label for {source:?}"
            );
        }
    }
}
