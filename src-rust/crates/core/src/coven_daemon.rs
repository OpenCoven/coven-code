//! Tier B daemon IPC — async-free HTTP-over-Unix-socket client.
//!
//! Talks to the Coven daemon at `~/.coven/coven.sock` using raw
//! `UnixStream` + hand-written HTTP/1.0 requests.  No tokio dependency
//! is added; all calls are blocking and degrade gracefully when the
//! daemon is absent.

#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[cfg(unix)]
use crate::coven_shared::coven_home;

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

/// Condensed view of a familiar's live status from the daemon.
#[derive(Debug, Clone)]
pub struct FamiliarStatus {
    pub id: String,
    pub display_name: String,
    pub emoji: String,
    pub status: String,
    pub active_sessions: u32,
    pub memory_freshness: String,
}

/// Condensed view of a running (non-archived) daemon session.
#[derive(Debug, Clone)]
pub struct DaemonSession {
    pub id: String,
    pub harness: String,
    pub title: String,
    pub status: String,
    pub project_root: String,
}

/// Payload for creating a new Coven daemon session.
#[derive(Debug, Clone, Serialize)]
pub struct CreateSessionRequest {
    pub familiar: String,
    pub project_root: String,
    pub harness: String,
    pub title: String,
    pub initial_message: String,
}

// ---------------------------------------------------------------------------
// Raw JSON shapes (private — only used for deserialization)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RawFamiliar {
    #[serde(default)]
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    emoji: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    active_sessions: Option<u32>,
    #[serde(default)]
    memory_freshness: Option<String>,
}

#[derive(Deserialize)]
struct RawSession {
    #[serde(default)]
    id: String,
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    project_root: Option<String>,
    #[serde(default)]
    archived_at: Option<String>,
}

// ---------------------------------------------------------------------------
// DaemonClient
// ---------------------------------------------------------------------------

/// Blocking HTTP-over-Unix-socket client for the Coven daemon.
pub struct DaemonClient {
    #[cfg(unix)]
    sock_path: PathBuf,
}

impl DaemonClient {
    /// Create a client targeting the default socket path.
    ///
    /// Returns `None` when the socket file does not exist (daemon is not
    /// running / not installed).  Never panics.
    pub fn new() -> Option<Self> {
        #[cfg(unix)]
        {
            let home = coven_home()?;
            let sock = home.join("coven.sock");
            if sock.exists() {
                Some(Self { sock_path: sock })
            } else {
                None
            }
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    // -- internal helpers ---------------------------------------------------

    /// Open a fresh `UnixStream` connection with a short timeout.
    #[cfg(unix)]
    fn connect(&self) -> std::io::Result<UnixStream> {
        let stream = UnixStream::connect(&self.sock_path)?;
        let timeout = Duration::from_millis(200);
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(stream)
    }

    /// Send a minimal HTTP/1.0 request and return the body string.
    ///
    /// HTTP/1.0 is used so the server closes the connection after the
    /// response — no need to parse `Content-Length` or chunked encoding.
    fn request(&self, method: &str, path: &str, body: Option<&str>) -> Option<String> {
        #[cfg(unix)]
        {
            let mut stream = self.connect().ok()?;
            let request = match body {
                Some(body) => format!(
                    "{method} {path} HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                ),
                None => format!(
                    "{method} {path} HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\n\r\n"
                ),
            };
            stream.write_all(request.as_bytes()).ok()?;
            stream.flush().ok()?;

            let mut raw = Vec::new();
            stream.read_to_end(&mut raw).ok()?;

            let response = String::from_utf8_lossy(&raw);

            // Split on the blank line that separates headers from body.
            if let Some(idx) = response.find("\r\n\r\n") {
                // Verify the response has a 2xx status code.
                let status_line = response.lines().next().unwrap_or("");
                let status_code = status_line.split_whitespace().nth(1)?.parse::<u16>().ok()?;
                if !(200..300).contains(&status_code) {
                    return None;
                }
                Some(response[idx + 4..].to_string())
            } else {
                None
            }
        }
        #[cfg(not(unix))]
        {
            let _ = method;
            let _ = path;
            let _ = body;
            None
        }
    }

    /// Send a minimal HTTP/1.0 GET and return the body string.
    fn get(&self, path: &str) -> Option<String> {
        self.request("GET", path, None)
    }

    // -- public API ---------------------------------------------------------

    /// Quick liveness check — returns `true` if the daemon responds with 200.
    pub fn is_online(&self) -> bool {
        self.get("/api/v1/familiars").is_some()
    }

    /// Fetch all familiar statuses.  Returns an empty `Vec` on any error.
    pub fn familiar_statuses(&self) -> Vec<FamiliarStatus> {
        let body = match self.get("/api/v1/familiars") {
            Some(b) => b,
            None => return Vec::new(),
        };
        let raw: Vec<RawFamiliar> = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        raw.into_iter()
            .map(|r| FamiliarStatus {
                display_name: r.display_name.unwrap_or_else(|| r.id.clone()),
                emoji: r.emoji.unwrap_or_default(),
                status: r.status.unwrap_or_else(|| "unknown".to_string()),
                active_sessions: r.active_sessions.unwrap_or(0),
                memory_freshness: r.memory_freshness.unwrap_or_default(),
                id: r.id,
            })
            .collect()
    }

    /// Fetch non-archived sessions.  Returns an empty `Vec` on any error.
    pub fn active_sessions(&self) -> Vec<DaemonSession> {
        let body = match self.get("/api/v1/sessions") {
            Some(b) => b,
            None => return Vec::new(),
        };
        let raw: Vec<RawSession> = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        raw.into_iter()
            .filter(|r| r.archived_at.is_none())
            .map(|r| DaemonSession {
                harness: r.harness.unwrap_or_default(),
                title: r.title.unwrap_or_default(),
                status: r.status.unwrap_or_else(|| "unknown".to_string()),
                project_root: r.project_root.unwrap_or_default(),
                id: r.id,
            })
            .collect()
    }

    /// Create a daemon session and return its session id.
    pub fn create_session(&self, req: CreateSessionRequest) -> Result<String, String> {
        let body = serde_json::to_string(&req)
            .map_err(|e| format!("Failed to encode daemon session request: {e}"))?;
        let response = self
            .request("POST", "/api/v1/sessions", Some(&body))
            .ok_or_else(|| {
                "Coven daemon did not return a successful session response".to_string()
            })?;
        let value: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| format!("Coven daemon returned invalid session JSON: {e}"))?;
        value
            .get("id")
            .or_else(|| value.get("session_id"))
            .or_else(|| value.get("sessionId"))
            .and_then(|id| id.as_str())
            .map(|id| id.to_string())
            .ok_or_else(|| "Coven daemon response did not include a session id".to_string())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coven_shared::COVEN_HOME_ENV_LOCK;
    use std::fs;

    /// Guard that temporarily sets `COVEN_HOME` and restores it on drop.
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }
    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, val);
            Self { key, original }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn new_returns_none_when_sock_absent() {
        let _lock = COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let _g = EnvGuard::set("COVEN_HOME", dir.path().to_str().unwrap());
        // Directory exists but no coven.sock inside → should return None.
        assert!(DaemonClient::new().is_none());
    }

    #[test]
    fn new_returns_some_when_sock_present() {
        let _lock = COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // Create a placeholder file (not a real socket, just needs to exist).
        fs::write(dir.path().join("coven.sock"), b"").unwrap();
        let _g = EnvGuard::set("COVEN_HOME", dir.path().to_str().unwrap());
        assert!(DaemonClient::new().is_some());
    }

    #[test]
    fn familiar_status_deserializes_from_json() {
        let json = r#"[
            {
                "id": "sage",
                "display_name": "Sage",
                "emoji": "🌿",
                "role": "researcher",
                "description": "Deep research familiar",
                "status": "active",
                "active_sessions": 2,
                "memory_freshness": "fresh"
            },
            {
                "id": "kitty",
                "status": "idle",
                "active_sessions": 0
            }
        ]"#;

        let raw: Vec<RawFamiliar> = serde_json::from_str(json).unwrap();
        assert_eq!(raw.len(), 2);

        let s0 = FamiliarStatus {
            display_name: raw[0]
                .display_name
                .clone()
                .unwrap_or_else(|| raw[0].id.clone()),
            emoji: raw[0].emoji.clone().unwrap_or_default(),
            status: raw[0].status.clone().unwrap_or_default(),
            active_sessions: raw[0].active_sessions.unwrap_or(0),
            memory_freshness: raw[0].memory_freshness.clone().unwrap_or_default(),
            id: raw[0].id.clone(),
        };
        assert_eq!(s0.id, "sage");
        assert_eq!(s0.display_name, "Sage");
        assert_eq!(s0.emoji, "🌿");
        assert_eq!(s0.status, "active");
        assert_eq!(s0.active_sessions, 2);

        let s1 = FamiliarStatus {
            display_name: raw[1]
                .display_name
                .clone()
                .unwrap_or_else(|| raw[1].id.clone()),
            emoji: raw[1].emoji.clone().unwrap_or_default(),
            status: raw[1].status.clone().unwrap_or_default(),
            active_sessions: raw[1].active_sessions.unwrap_or(0),
            memory_freshness: raw[1].memory_freshness.clone().unwrap_or_default(),
            id: raw[1].id.clone(),
        };
        assert_eq!(s1.id, "kitty");
        assert_eq!(s1.display_name, "kitty"); // falls back to id
        assert_eq!(s1.active_sessions, 0);
    }

    #[test]
    fn familiar_statuses_returns_empty_on_bad_json() {
        let _lock = COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // Placeholder sock — not a real socket, so connect() will fail.
        fs::write(dir.path().join("coven.sock"), b"").unwrap();
        let _g = EnvGuard::set("COVEN_HOME", dir.path().to_str().unwrap());
        let client = DaemonClient::new().unwrap();
        // connect() will fail → familiar_statuses() must return empty vec, not panic.
        assert!(client.familiar_statuses().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn create_session_posts_payload_and_returns_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            assert!(request.starts_with("POST /api/v1/sessions HTTP/1.0"));
            assert!(request.contains("Host: localhost\r\n"));
            assert!(request.contains("Content-Type: application/json\r\n"));
            assert!(request.contains("\"familiar\":\"sage\""));
            assert!(request.contains("\"project_root\":\"/tmp/project\""));
            assert!(request.contains("\"initial_message\":\"handoff context\""));
            stream
                .write_all(
                    b"HTTP/1.0 201 Created\r\nContent-Type: application/json\r\n\r\n{\"id\":\"sess_123\"}",
                )
                .unwrap();
        });

        let client = DaemonClient { sock_path: sock };
        let session_id = client
            .create_session(CreateSessionRequest {
                familiar: "sage".to_string(),
                project_root: "/tmp/project".to_string(),
                harness: "openclaw".to_string(),
                title: "Handoff from coven-code".to_string(),
                initial_message: "handoff context".to_string(),
            })
            .unwrap();

        server.join().unwrap();
        assert_eq!(session_id, "sess_123");
    }
}
