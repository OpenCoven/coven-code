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
// `Duration` appears in the public `check_reachability(timeout)` signature,
// so its import can't be gated on `cfg(unix)` even though the only real
// implementation lives there.
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

/// Condensed view of a daemon session.
#[derive(Debug, Clone)]
pub struct DaemonSession {
    pub id: String,
    pub harness: String,
    pub title: String,
    pub status: String,
    pub project_root: String,
    /// Present when the session has been archived; `None` for live sessions.
    pub archived_at: Option<String>,
}

/// Daemon health response — surfaced by `GET /api/v1/health`.
#[derive(Debug, Clone)]
pub struct DaemonHealth {
    pub api_version: String,
    pub coven_version: String,
    pub pid: Option<u32>,
    pub socket: Option<String>,
    pub started_at: Option<String>,
}

/// Page of session events returned by `GET /api/v1/sessions/:id/events`.
#[derive(Debug, Clone)]
pub struct EventPage {
    pub events: Vec<EventRecord>,
    pub next_after_seq: Option<i64>,
    pub has_more: bool,
}

/// One session event from the daemon's event ledger.
#[derive(Debug, Clone)]
pub struct EventRecord {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub seq: Option<i64>,
    pub created_at: String,
    pub payload_json: String,
}

/// Result of `POST /api/v1/actions`. Mirrors the daemon's `ControlActionResponse`.
#[derive(Debug, Clone)]
pub struct ControlActionResult {
    pub ok: bool,
    pub accepted: bool,
    pub action: String,
    pub status: String,
    pub reason: Option<String>,
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
// Errors
// ---------------------------------------------------------------------------

/// Structured error returned by [`DaemonClient`] methods.
///
/// The daemon's HTTP contract (`coven.daemon.v1`) wraps every error in
/// `{ "error": { "code": "...", "message": "...", "details": {...} } }`.
/// We surface those fields here so user-facing layers can render
/// actionable messages like `Session not running (session_not_live)`
/// instead of a generic `daemon offline`.
#[derive(Debug, Clone)]
pub enum DaemonError {
    /// Socket file is missing or the daemon refused the connection.
    Offline { reason: String },
    /// Connection succeeded but the request/response cycle failed at the
    /// transport layer (read/write timed out, peer closed mid-stream, etc.).
    Transport(String),
    /// HTTP completed but the daemon returned a non-2xx status. `code`
    /// carries the structured error code when the daemon supplied one,
    /// `message` carries the human-readable hint.
    BadStatus {
        status: u16,
        code: Option<String>,
        message: Option<String>,
    },
    /// Response body could not be parsed as JSON / the expected shape.
    MalformedResponse(String),
    /// Response was well-formed but a required field was missing.
    MissingField(&'static str),
}

impl DaemonError {
    /// Return the structured error code when one is present.
    pub fn code(&self) -> Option<&str> {
        match self {
            DaemonError::BadStatus { code, .. } => code.as_deref(),
            _ => None,
        }
    }

    /// Return the HTTP status when this is a bad-status error.
    pub fn status(&self) -> Option<u16> {
        match self {
            DaemonError::BadStatus { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// `true` when the error indicates the daemon is not reachable at all
    /// (socket missing, connection refused). UI layers can use this to
    /// decide whether to suggest `coven daemon start`.
    pub fn is_offline(&self) -> bool {
        matches!(self, DaemonError::Offline { .. })
    }
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::Offline { reason } => write!(f, "Coven daemon offline ({reason})"),
            DaemonError::Transport(msg) => write!(f, "daemon transport error: {msg}"),
            DaemonError::BadStatus {
                status,
                code,
                message,
            } => match (code.as_deref(), message.as_deref()) {
                (Some(c), Some(m)) => write!(f, "{m} ({status} {c})"),
                (Some(c), None) => write!(f, "daemon returned {status} ({c})"),
                (None, Some(m)) => write!(f, "{m} ({status})"),
                (None, None) => write!(f, "daemon returned {status}"),
            },
            DaemonError::MalformedResponse(msg) => {
                write!(f, "daemon returned invalid response: {msg}")
            }
            DaemonError::MissingField(name) => write!(f, "daemon response missing field `{name}`"),
        }
    }
}

impl std::error::Error for DaemonError {}

// ---------------------------------------------------------------------------
// DaemonReachability
// ---------------------------------------------------------------------------

/// Three-valued result for a lightweight "is the daemon reachable?" probe.
///
/// [`DaemonClient::is_online`] collapses `TimedOut` and `Offline` into the
/// same `false`, which is fine for a quick polling indicator but lies on a
/// busy daemon — see issue #50. Callers that paint a sticky status line
/// (e.g. the welcome screen) should call [`DaemonClient::check_reachability`]
/// instead and treat `TimedOut` as "best-effort: still online".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonReachability {
    /// Socket connected and the daemon responded within the timeout budget.
    Online,
    /// Socket connected but the daemon didn't respond in time. The daemon
    /// is almost certainly alive — it's just busy serving a different
    /// request. Don't render this as "offline".
    TimedOut,
    /// Socket file is missing or connect was actively refused — the daemon
    /// genuinely isn't running.
    Offline,
}

impl DaemonReachability {
    /// `true` for both `Online` and `TimedOut` — i.e. anything that isn't
    /// a confirmed offline state.
    pub fn looks_alive(self) -> bool {
        !matches!(self, DaemonReachability::Offline)
    }
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

    /// Default per-call timeout for the quick polling path
    /// ([`is_online`], the high-frequency status-row indicator).
    /// Chosen short so a missing daemon shows up as offline in < 1
    /// frame; callers that need correctness over latency should use
    /// [`Self::check_reachability`] with an explicit budget.
    ///
    /// Not gated on `cfg(unix)`: [`Self::is_online`] references it on every
    /// platform, even though the non-unix [`Self::check_reachability`] path
    /// ignores the timeout and reports `Offline` outright.
    const DEFAULT_TIMEOUT_MS: u64 = 200;

    /// Per-call timeout for substantive request/response verbs routed through
    /// [`Self::request`] (`create_session`, `session_log`, `session_events`,
    /// `capabilities`, `control_action`, …). These can legitimately take
    /// longer than the [`Self::DEFAULT_TIMEOUT_MS`] poll budget on a busy
    /// daemon, so they get a wider budget to avoid spurious `Transport`
    /// timeouts. The quick-poll path ([`Self::is_online`]) keeps the short
    /// budget via [`Self::check_reachability`].
    #[cfg(unix)]
    const REQUEST_TIMEOUT_MS: u64 = 5000;

    /// Open a fresh `UnixStream` connection with the request timeout budget.
    #[cfg(unix)]
    fn connect(&self) -> std::io::Result<UnixStream> {
        self.connect_with_timeout(Duration::from_millis(Self::REQUEST_TIMEOUT_MS))
    }

    /// Open a fresh `UnixStream` connection with a caller-supplied timeout.
    #[cfg(unix)]
    fn connect_with_timeout(&self, timeout: Duration) -> std::io::Result<UnixStream> {
        let stream = UnixStream::connect(&self.sock_path)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(stream)
    }

    /// Send a minimal HTTP/1.0 request and return the body string.
    ///
    /// HTTP/1.0 is used so the server closes the connection after the
    /// response — no need to parse `Content-Length` or chunked encoding.
    /// Non-2xx responses are surfaced via [`DaemonError::BadStatus`] with
    /// the daemon's structured error envelope parsed out.
    fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<String, DaemonError> {
        #[cfg(unix)]
        {
            let mut stream = self.connect().map_err(|e| DaemonError::Offline {
                reason: e.to_string(),
            })?;
            let request = match body {
                Some(body) => format!(
                    "{method} {path} HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                ),
                None => format!(
                    "{method} {path} HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\n\r\n"
                ),
            };
            stream
                .write_all(request.as_bytes())
                .map_err(|e| DaemonError::Transport(e.to_string()))?;
            stream
                .flush()
                .map_err(|e| DaemonError::Transport(e.to_string()))?;

            let mut raw = Vec::new();
            stream
                .read_to_end(&mut raw)
                .map_err(|e| DaemonError::Transport(e.to_string()))?;

            let response = String::from_utf8_lossy(&raw);
            let idx = response.find("\r\n\r\n").ok_or_else(|| {
                DaemonError::MalformedResponse("missing header/body delimiter".to_string())
            })?;

            let status_line = response.lines().next().unwrap_or("");
            let status_code = status_line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse::<u16>().ok())
                .ok_or_else(|| {
                    DaemonError::MalformedResponse(format!(
                        "could not parse status from `{status_line}`"
                    ))
                })?;

            let body_str = response[idx + 4..].to_string();

            if (200..300).contains(&status_code) {
                Ok(body_str)
            } else {
                // Try to extract the structured envelope:
                // { "error": { "code": "...", "message": "..." } }
                let (code, message) = match serde_json::from_str::<serde_json::Value>(&body_str) {
                    Ok(v) => {
                        let err = v.get("error");
                        let code = err
                            .and_then(|e| e.get("code"))
                            .and_then(|c| c.as_str())
                            .map(str::to_string);
                        let message = err
                            .and_then(|e| e.get("message"))
                            .and_then(|c| c.as_str())
                            .map(str::to_string);
                        (code, message)
                    }
                    Err(_) => (None, None),
                };
                Err(DaemonError::BadStatus {
                    status: status_code,
                    code,
                    message,
                })
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (method, path, body);
            Err(DaemonError::Offline {
                reason: "unix sockets unavailable on this platform".to_string(),
            })
        }
    }

    /// Send a minimal HTTP/1.0 GET and return the body string.
    fn get(&self, path: &str) -> Result<String, DaemonError> {
        self.request("GET", path, None)
    }

    // -- public API ---------------------------------------------------------

    /// Quick liveness check — returns `true` if the daemon responds with 200
    /// to `GET /api/v1/familiars`. Discards the structured-error envelope on
    /// purpose; use [`Self::health`] for richer diagnostics. Uses the
    /// default short timeout; callers that need to distinguish "offline"
    /// from "busy" should use [`Self::check_reachability`] instead.
    pub fn is_online(&self) -> bool {
        matches!(
            self.check_reachability(Duration::from_millis(Self::DEFAULT_TIMEOUT_MS)),
            DaemonReachability::Online
        )
    }

    /// Three-valued reachability probe. Spends up to `timeout` waiting for
    /// the daemon to respond and reports whether the failure (if any) was
    /// a hard offline state or a transient timeout.
    ///
    /// Welcome-screen status indicators should call this with a longer
    /// budget (~2 s) so a busy daemon doesn't flicker to "offline" — see
    /// issue #50. The status-row polling indicator can keep using
    /// [`Self::is_online`] with the default short timeout.
    pub fn check_reachability(&self, timeout: Duration) -> DaemonReachability {
        #[cfg(unix)]
        {
            use std::io::{Read, Write};

            let mut stream = match self.connect_with_timeout(timeout) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    return DaemonReachability::TimedOut;
                }
                Err(_) => return DaemonReachability::Offline,
            };

            let request =
                "GET /api/v1/familiars HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\n\r\n";
            if let Err(e) = stream.write_all(request.as_bytes()) {
                return match e.kind() {
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                        DaemonReachability::TimedOut
                    }
                    _ => DaemonReachability::Offline,
                };
            }
            if let Err(e) = stream.flush() {
                return match e.kind() {
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                        DaemonReachability::TimedOut
                    }
                    _ => DaemonReachability::Offline,
                };
            }

            // We only need to see the response start arriving — we don't
            // care about parsing it. Read one byte (or hit timeout) and
            // call it.
            let mut peek = [0_u8; 1];
            match stream.read(&mut peek) {
                Ok(0) => DaemonReachability::Offline, // peer closed before any byte
                Ok(_) => DaemonReachability::Online,
                Err(e) => match e.kind() {
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                        DaemonReachability::TimedOut
                    }
                    _ => DaemonReachability::Offline,
                },
            }
        }
        #[cfg(not(unix))]
        {
            let _ = timeout;
            DaemonReachability::Offline
        }
    }

    fn parse_session_list(body: &str) -> Result<Vec<DaemonSession>, DaemonError> {
        let raw: Vec<RawSession> = serde_json::from_str(body)
            .map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
        Ok(raw
            .into_iter()
            .map(|r| DaemonSession {
                harness: r.harness.unwrap_or_default(),
                title: r.title.unwrap_or_default(),
                status: r.status.unwrap_or_else(|| "unknown".to_string()),
                project_root: r.project_root.unwrap_or_default(),
                archived_at: r.archived_at.clone(),
                id: r.id,
            })
            .collect())
    }

    /// Fetch all familiar statuses.
    pub fn familiar_statuses(&self) -> Result<Vec<FamiliarStatus>, DaemonError> {
        let body = self.get("/api/v1/familiars")?;
        let raw: Vec<RawFamiliar> = serde_json::from_str(&body)
            .map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
        Ok(raw
            .into_iter()
            .map(|r| FamiliarStatus {
                display_name: r.display_name.unwrap_or_else(|| r.id.clone()),
                emoji: r.emoji.unwrap_or_default(),
                status: r.status.unwrap_or_else(|| "unknown".to_string()),
                active_sessions: r.active_sessions.unwrap_or(0),
                memory_freshness: r.memory_freshness.unwrap_or_default(),
                id: r.id,
            })
            .collect())
    }

    /// Fetch non-archived sessions.
    pub fn active_sessions(&self) -> Result<Vec<DaemonSession>, DaemonError> {
        let body = self.get("/api/v1/sessions")?;
        Ok(Self::parse_session_list(&body)?
            .into_iter()
            .filter(|s| s.archived_at.is_none())
            .collect())
    }

    /// Fetch every session known to the daemon, including archived ones.
    pub fn all_sessions(&self) -> Result<Vec<DaemonSession>, DaemonError> {
        let body = self.get("/api/v1/sessions")?;
        Self::parse_session_list(&body)
    }

    /// Fetch a single session by id.
    pub fn get_session(&self, session_id: &str) -> Result<DaemonSession, DaemonError> {
        let path = format!("/api/v1/sessions/{}", url_quote(session_id));
        let body = self.get(&path)?;
        let r: RawSession = serde_json::from_str(&body)
            .map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
        if r.id.is_empty() {
            return Err(DaemonError::MissingField("id"));
        }
        Ok(DaemonSession {
            harness: r.harness.unwrap_or_default(),
            title: r.title.unwrap_or_default(),
            status: r.status.unwrap_or_else(|| "unknown".to_string()),
            project_root: r.project_root.unwrap_or_default(),
            archived_at: r.archived_at.clone(),
            id: r.id,
        })
    }

    /// Daemon liveness + version metadata.
    pub fn health(&self) -> Result<DaemonHealth, DaemonError> {
        let body = self.get("/api/v1/health")?;
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
        let daemon = value.get("daemon");
        Ok(DaemonHealth {
            api_version: value
                .get("apiVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            coven_version: value
                .get("covenVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            pid: daemon
                .and_then(|d| d.get("pid"))
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            socket: daemon
                .and_then(|d| d.get("socket"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            started_at: daemon
                .and_then(|d| d.get("startedAt"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    }

    /// Forward `input` to a live PTY-backed session.
    pub fn send_input(&self, session_id: &str, input: &str) -> Result<(), DaemonError> {
        let body = serde_json::to_string(&serde_json::json!({ "input": input }))
            .map_err(|e| DaemonError::MalformedResponse(format!("encode input: {e}")))?;
        let path = format!("/api/v1/sessions/{}/input", url_quote(session_id));
        self.request("POST", &path, Some(&body)).map(|_| ())
    }

    /// Terminate a live session.
    pub fn kill_session(&self, session_id: &str) -> Result<(), DaemonError> {
        let path = format!("/api/v1/sessions/{}/kill", url_quote(session_id));
        self.request("POST", &path, Some("{}")).map(|_| ())
    }

    /// Fetch the redacted log preview for a session.
    pub fn session_log(&self, session_id: &str) -> Result<String, DaemonError> {
        let path = format!("/api/v1/sessions/{}/log", url_quote(session_id));
        self.get(&path)
    }

    /// Fetch the daemon's capability catalog. Returns the raw JSON body so
    /// callers can pretty-print it without us tying the client to one shape.
    pub fn capabilities(&self) -> Result<String, DaemonError> {
        self.get("/api/v1/capabilities")
    }

    /// Fetch a page of session events. Uses the documented `afterSeq` cursor
    /// pagination.
    pub fn session_events(
        &self,
        session_id: &str,
        after_seq: Option<i64>,
        limit: Option<u32>,
    ) -> Result<EventPage, DaemonError> {
        let mut path = format!("/api/v1/sessions/{}/events", url_quote(session_id));
        let mut query: Vec<String> = Vec::new();
        if let Some(seq) = after_seq {
            query.push(format!("afterSeq={seq}"));
        }
        if let Some(n) = limit {
            query.push(format!("limit={n}"));
        }
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query.join("&"));
        }
        let body = self.get(&path)?;
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
        let events_json = value
            .get("events")
            .and_then(|v| v.as_array())
            .ok_or(DaemonError::MissingField("events"))?;
        let events: Vec<EventRecord> = events_json
            .iter()
            .map(|ev| EventRecord {
                id: ev
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                session_id: ev
                    .get("sessionId")
                    .or_else(|| ev.get("session_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                kind: ev
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                seq: ev.get("seq").and_then(|v| v.as_i64()),
                created_at: ev
                    .get("createdAt")
                    .or_else(|| ev.get("created_at"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                payload_json: ev
                    .get("payload")
                    .or_else(|| ev.get("payloadJson"))
                    .or_else(|| ev.get("payload_json"))
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            })
            .collect();
        let next_after_seq = value
            .get("nextCursor")
            .and_then(|c| c.get("afterSeq"))
            .and_then(|v| v.as_i64());
        let has_more = value
            .get("hasMore")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(EventPage {
            events,
            next_after_seq,
            has_more,
        })
    }

    /// Send a control-plane action. `args` is an optional JSON object that
    /// will be included as the request's `args` field if present.
    pub fn control_action(
        &self,
        action: &str,
        args: Option<serde_json::Value>,
    ) -> Result<ControlActionResult, DaemonError> {
        let mut payload = serde_json::Map::new();
        payload.insert(
            "action".to_string(),
            serde_json::Value::String(action.to_string()),
        );
        if let Some(a) = args {
            payload.insert("args".to_string(), a);
        }
        let body = serde_json::Value::Object(payload).to_string();
        let response = self.request("POST", "/api/v1/actions", Some(&body))?;
        let value: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
        Ok(ControlActionResult {
            ok: value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            accepted: value
                .get("accepted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            action: value
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or(action)
                .to_string(),
            status: value
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            reason: value
                .get("reason")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    }

    /// Create a daemon session and return its session id.
    pub fn create_session(&self, req: CreateSessionRequest) -> Result<String, DaemonError> {
        let body = serde_json::to_string(&req)
            .map_err(|e| DaemonError::MalformedResponse(format!("encode request: {e}")))?;
        let response = self.request("POST", "/api/v1/sessions", Some(&body))?;
        let value: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
        value
            .get("id")
            .or_else(|| value.get("session_id"))
            .or_else(|| value.get("sessionId"))
            .and_then(|id| id.as_str())
            .map(|id| id.to_string())
            .ok_or(DaemonError::MissingField("id"))
    }
}

/// Minimal percent-encoder for path segments — escapes anything outside the
/// unreserved set defined by RFC 3986. The daemon's session ids are UUID-shaped
/// today, but encoding defensively avoids surprises if that ever changes.
fn url_quote(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        let b = *byte;
        let is_unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if is_unreserved {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coven_shared::COVEN_HOME_ENV_LOCK;
    #[cfg(unix)]
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

    #[cfg(unix)]
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
                "id": "researcher",
                "display_name": "Researcher",
                "emoji": "🌿",
                "role": "researcher",
                "description": "Deep research familiar",
                "status": "active",
                "active_sessions": 2,
                "memory_freshness": "fresh"
            },
            {
                "id": "helper",
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
        assert_eq!(s0.id, "researcher");
        assert_eq!(s0.display_name, "Researcher");
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
        assert_eq!(s1.id, "helper");
        assert_eq!(s1.display_name, "helper"); // falls back to id
        assert_eq!(s1.active_sessions, 0);
    }

    #[cfg(unix)]
    #[test]
    fn familiar_statuses_returns_offline_when_connect_fails() {
        let _lock = COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // Placeholder sock — not a real socket, so connect() will fail.
        fs::write(dir.path().join("coven.sock"), b"").unwrap();
        let _g = EnvGuard::set("COVEN_HOME", dir.path().to_str().unwrap());
        let client = DaemonClient::new().unwrap();
        // connect() will fail → familiar_statuses() must surface a structured
        // Offline error, not silently empty.
        match client.familiar_statuses() {
            Err(DaemonError::Offline { .. }) => {}
            other => panic!("expected Offline, got {other:?}"),
        }
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
            assert!(request.contains("\"familiar\":\"researcher\""));
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
                familiar: "researcher".to_string(),
                project_root: "/tmp/project".to_string(),
                harness: "openclaw".to_string(),
                title: "Handoff from coven-code".to_string(),
                initial_message: "handoff context".to_string(),
            })
            .unwrap();

        server.join().unwrap();
        assert_eq!(session_id, "sess_123");
    }

    #[cfg(unix)]
    #[test]
    fn send_input_surfaces_session_not_live_envelope() {
        // The daemon's 409 response carries a structured error code; we must
        // surface it through DaemonError::BadStatus so /coven send can render
        // "Session is not running (session_not_live)" instead of generic
        // "daemon offline".
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf).unwrap();
            stream
                .write_all(
                    b"HTTP/1.0 409 Conflict\r\nContent-Type: application/json\r\n\r\n\
                      {\"error\":{\"code\":\"session_not_live\",\
                      \"message\":\"Session is not running.\"}}",
                )
                .unwrap();
        });

        let client = DaemonClient { sock_path: sock };
        let err = client.send_input("dead-session", "hi").unwrap_err();
        server.join().unwrap();

        match err {
            DaemonError::BadStatus {
                status,
                code,
                message,
            } => {
                assert_eq!(status, 409);
                assert_eq!(code.as_deref(), Some("session_not_live"));
                assert_eq!(message.as_deref(), Some("Session is not running."));
            }
            other => panic!("expected BadStatus(409 session_not_live), got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn get_session_surfaces_404_session_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf).unwrap();
            stream
                .write_all(
                    b"HTTP/1.0 404 Not Found\r\nContent-Type: application/json\r\n\r\n\
                      {\"error\":{\"code\":\"session_not_found\",\
                      \"message\":\"Session was not found.\"}}",
                )
                .unwrap();
        });

        let client = DaemonClient { sock_path: sock };
        let err = client.get_session("ghost").unwrap_err();
        server.join().unwrap();

        assert_eq!(err.status(), Some(404));
        assert_eq!(err.code(), Some("session_not_found"));
        assert!(!err.is_offline());
    }

    #[test]
    fn offline_error_is_marked_offline() {
        let err = DaemonError::Offline {
            reason: "ENOENT".to_string(),
        };
        assert!(err.is_offline());
        assert!(err.code().is_none());
        assert!(err.status().is_none());
    }

    /// Issue #50: a real reachability probe against a daemon that has
    /// accepted the connection but doesn't write any bytes back within
    /// the budget must return `TimedOut`, not `Offline`. Renderers like
    /// the welcome panel can then treat the daemon as still alive
    /// instead of flickering to "offline" during heavy load.
    #[cfg(unix)]
    #[test]
    fn issue_50_check_reachability_distinguishes_timeout_from_offline() {
        use std::os::unix::net::UnixListener;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        // Slow-daemon scenario: accept the connection but never write a
        // response. The client must time out within its budget.
        let _server = std::thread::spawn(move || {
            // Hold the connection open for longer than any sane test
            // timeout so the client is forced into TimedOut.
            let (_stream, _) = match listener.accept() {
                Ok(p) => p,
                Err(_) => return,
            };
            std::thread::sleep(Duration::from_secs(5));
        });

        let client = DaemonClient { sock_path: sock };
        // Probe with a 200 ms budget — well under the server's 5 s hold.
        let result = client.check_reachability(Duration::from_millis(200));
        assert_eq!(
            result,
            DaemonReachability::TimedOut,
            "slow daemon must be TimedOut, not Offline"
        );
        assert!(
            result.looks_alive(),
            "TimedOut must count as 'looks alive' so the welcome stays online"
        );
    }

    /// Reachability against a tempdir with no socket file at all must
    /// return `Offline` — the absent-socket case is the only path that
    /// should ever paint "Daemon: offline" in the welcome.
    #[cfg(unix)]
    #[test]
    fn issue_50_check_reachability_offline_when_socket_absent() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("definitely-not-here.sock");
        let client = DaemonClient { sock_path: sock };
        let result = client.check_reachability(std::time::Duration::from_millis(200));
        assert_eq!(result, DaemonReachability::Offline);
        assert!(!result.looks_alive(), "Offline must NOT count as alive");
    }

    /// A daemon that accepts and responds with a 200 OK within the
    /// budget must return `Online`.
    #[cfg(unix)]
    #[test]
    fn issue_50_check_reachability_online_when_daemon_responds() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        let _server = std::thread::spawn(move || {
            let (mut stream, _) = match listener.accept() {
                Ok(p) => p,
                Err(_) => return,
            };
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);
            let _ =
                stream.write_all(b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n[]");
        });

        let client = DaemonClient { sock_path: sock };
        let result = client.check_reachability(std::time::Duration::from_secs(2));
        assert_eq!(result, DaemonReachability::Online);
    }
}
