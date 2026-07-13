//! Best-effort registration of interactive sessions in the Coven daemon ledger.
//!
//! Every failure is swallowed to a debug log — a dead or absent daemon must
//! never affect the TUI.

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
