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

/// How long each health read around a registration may wait.
#[cfg(unix)]
const INSTANCE_READ_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// Register a session; `Ok` only when the daemon accepted it. Callers may
/// ignore the result, but must not treat registration as daemon control.
///
/// `Ok(Some(instance))` names the daemon that took the registration: the
/// same instance answered health before and after the request. `Ok(None)`
/// means it was accepted but no single instance can be named for it (the
/// daemon changed during the request, or did not identify itself), so a
/// later restart could not be told from this daemon.
#[cfg(unix)]
pub fn notify_session_start(
    id: &str,
    project_root: &std::path::Path,
    title: &str,
) -> Result<Option<crate::coven_daemon::DaemonInstance>, String> {
    use crate::coven_daemon::DaemonSighting;

    let Some(client) = crate::coven_daemon::DaemonClient::new() else {
        return Err("no Coven daemon is configured".to_string());
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
    let before = client.sight(INSTANCE_READ_BUDGET);
    client.register_external_session(&req).map_err(|e| {
        tracing::debug!("coven ledger register failed (ignored): {e}");
        e.to_string()
    })?;
    Ok(match (before, client.sight(INSTANCE_READ_BUDGET)) {
        (DaemonSighting::Instance(before), DaemonSighting::Instance(after)) if before == after => {
            Some(after)
        }
        _ => None,
    })
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
pub fn notify_session_start(
    _id: &str,
    _project_root: &std::path::Path,
    _title: &str,
) -> Result<Option<crate::coven_daemon::DaemonInstance>, String> {
    Err("the Coven daemon ledger is not supported on this platform".to_string())
}

#[cfg(not(unix))]
pub fn notify_session_complete(_id: &str, _exit_code: Option<i32>) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// #412, policy B: a registration is tied to the daemon instance that
    /// answered health before and after it, and to none when that changed
    /// in between or the daemon did not identify itself.
    #[cfg(unix)]
    #[test]
    fn a_registration_names_the_instance_that_took_it() {
        use crate::coven_daemon::fake::{health_response, FakeDaemon};
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::{Arc, Mutex};

        let _lock = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Puts `COVEN_HOME` back when the test ends, passing or not.
        struct RestoreCovenHome(Option<std::ffi::OsString>);
        impl Drop for RestoreCovenHome {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var("COVEN_HOME", value),
                    None => std::env::remove_var("COVEN_HOME"),
                }
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let _restore = RestoreCovenHome(std::env::var_os("COVEN_HOME"));
        std::env::set_var("COVEN_HOME", dir.path());
        let sock = dir.path().join("coven.sock");
        let accepted = "HTTP/1.0 201 Created\r\nContent-Type: application/json\r\n\r\n{}";
        let register = || notify_session_start("sess-1", dir.path(), "a title");

        // One daemon throughout: the registration names it.
        let requests = Arc::new(Mutex::new(Vec::new()));
        let daemon = {
            let requests = Arc::clone(&requests);
            FakeDaemon::start(&sock, move |request| {
                let line = request.lines().next().unwrap_or("").to_string();
                requests.lock().unwrap().push(line);
                if request.starts_with("GET /api/v1/health") {
                    health_response(7, "2026-11-18T05:00:00Z")
                } else {
                    accepted.to_string()
                }
            })
        };
        let instance = register().expect("accepted").expect("one instance");
        assert_eq!(instance.pid, Some(7));
        assert_eq!(instance.started_at.as_deref(), Some("2026-11-18T05:00:00Z"));
        assert_eq!(
            *requests.lock().unwrap(),
            [
                "GET /api/v1/health HTTP/1.0",
                "POST /api/v1/sessions/external HTTP/1.0",
                "GET /api/v1/health HTTP/1.0",
            ]
        );
        drop(daemon);
        std::fs::remove_file(&sock).unwrap();

        // The daemon answering health afterwards is not the one from before.
        let healths = AtomicU32::new(0);
        let daemon = FakeDaemon::start(&sock, move |request| {
            if request.starts_with("GET /api/v1/health") {
                health_response(
                    8 + healths.fetch_add(1, Ordering::AcqRel),
                    "2026-11-18T05:00:00Z",
                )
            } else {
                accepted.to_string()
            }
        });
        assert_eq!(register(), Ok(None), "a daemon that changed is not named");
        drop(daemon);
        std::fs::remove_file(&sock).unwrap();

        // Accepted by a daemon whose health names no process.
        let daemon = FakeDaemon::start(&sock, move |_| accepted.to_string());
        assert_eq!(register(), Ok(None), "an unidentified daemon is not named");
        drop(daemon);

        // The socket file is stale: nothing is registered.
        assert!(register().is_err());
    }

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
