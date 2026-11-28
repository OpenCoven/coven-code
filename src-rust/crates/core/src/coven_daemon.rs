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
#[cfg(unix)]
use std::time::Instant;

use serde::{Deserialize, Serialize};

#[cfg(unix)]
use crate::coven_shared::daemon_socket_path;

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
    /// Present when the session has been archived; `None` otherwise. Archive
    /// is separate from `status`: a finished session stays unarchived until
    /// someone archives it.
    pub archived_at: Option<String>,
}

impl DaemonSession {
    /// Whether the session is still live: its status is one of the daemon
    /// contract's nonterminal harness-session statuses, `created`, `running`
    /// or `idle`. The terminal ones (`completed`, `failed`, `killed`,
    /// `orphaned`) and any status the contract does not list are not live.
    pub fn is_live(&self) -> bool {
        matches!(self.status.as_str(), "created" | "running" | "idle")
    }
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

/// One running daemon, as far as this client can tell it from another: the
/// process its health response names and the socket file it was reached
/// through. Two readings that differ in any field are different instances.
/// [`DaemonClient::sight`] reports one only when the health response names
/// a `pid`, a `started_at` or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonInstance {
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub api_version: String,
    pub coven_version: String,
    /// Device and inode of the socket file, read before connecting.
    pub socket_file: Option<(u64, u64)>,
}

/// What one look at the daemon socket found. See [`DaemonClient::sight`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonSighting {
    /// No socket file, the connection was refused, or it closed with no
    /// answer: no daemon is serving.
    Gone,
    /// Something accepted the connection, but did not say which process it
    /// is: no complete answer within the budget (a busy daemon), an error
    /// status, or a health body naming neither a pid nor a start time.
    Unidentified,
    /// A daemon answered and named its process.
    Instance(DaemonInstance),
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

/// Payload for registering an externally-owned running session in the daemon.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterExternalSession {
    pub id: String,
    pub project_root: String,
    pub harness: String,
    pub title: String,
    pub transcript_path: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

// ---------------------------------------------------------------------------
// Raw JSON shapes (private — only used for deserialization)
// ---------------------------------------------------------------------------

/// One familiar-memory row from `GET /api/v1/memory`, as the terminal may
/// hold it. The relative `path` is never parsed. The title and excerpt are
/// kept only when the producer classified the entry and said no reveal is
/// required; otherwise they are dropped here, so no later layer can show,
/// log or export them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySummary {
    /// Opaque id from the daemon; not a content revision.
    pub id: String,
    /// Familiar directory name as the daemon reports it; not an identity.
    pub familiar_id: String,
    pub updated_at: Option<String>,
    pub source_label: Option<String>,
    /// The producer's verification label, as given; not an attestation.
    pub verification_state: String,
    pub preview: MemoryPreview,
}

/// What a list row may show of an entry's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryPreview {
    /// Unclassified, or the producer requires reveal: nothing is kept.
    Withheld,
    /// The producer classified the entry and said no reveal is required.
    Allowed { title: String, excerpt: String },
}

/// `GET /api/v1/memory/overview`. A missing count stays unknown, never zero;
/// a missing capability reads as absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryOverview {
    pub entries: Option<u64>,
    pub familiars: Option<u64>,
    pub last_updated_at: Option<String>,
    pub detail: bool,
    pub verification: bool,
    pub attestation_metadata: bool,
    pub supersession_history: bool,
    pub mutations: bool,
    /// The index verification state as given, such as `unavailable`.
    pub verification_state: Option<String>,
}

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
    /// The response has the shape of an older API than the one this client
    /// reads; the field named is the first that did not match. Carries no
    /// response content.
    UnsupportedShape(&'static str),
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
            DaemonError::UnsupportedShape(what) => {
                write!(f, "daemon response predates this API ({what})")
            }
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
    /// Create a client targeting [`crate::coven_shared::daemon_socket_path`]
    /// (`$COVEN_HOME/coven.sock`, else `~/.coven/coven.sock`).
    ///
    /// Returns `None` when the socket file does not exist (daemon is not
    /// running / not installed).  Never panics.
    pub fn new() -> Option<Self> {
        #[cfg(unix)]
        {
            let sock = daemon_socket_path()?;
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

    /// Whole budget for one substantive request/response verb routed through
    /// [`Self::request`] (`create_session`, `session_log`, `session_events`,
    /// `capabilities`, `control_action`, …): connecting, sending the request
    /// and reading the whole answer, so neither a daemon that never answers
    /// nor one that sends a byte now and then can hold a request longer
    /// (#580). These can legitimately take longer than the
    /// [`Self::DEFAULT_TIMEOUT_MS`] poll budget on a busy daemon, so they get
    /// a wider budget to avoid spurious `Transport` timeouts. The quick-poll
    /// path ([`Self::is_online`]) keeps the short budget via
    /// [`Self::check_reachability`].
    #[cfg(unix)]
    const REQUEST_TIMEOUT_MS: u64 = 5000;

    /// Connect to the daemon socket, giving up at `deadline`, and set the
    /// stream's read and write timeouts to the time left.
    ///
    /// `UnixStream::connect` takes no timeout. On Linux a connect to a socket
    /// whose listen queue is full waits for room for as long as the socket's
    /// send timeout allows, so that is set before connecting; macOS refuses
    /// such a connect at once.
    #[cfg(unix)]
    fn connect_by(&self, deadline: Instant) -> std::io::Result<UnixStream> {
        use socket2::{Domain, SockAddr, Socket, Type};
        use std::os::fd::OwnedFd;

        let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
        // socket2 passes a timeout under 1 µs as zero, which means none.
        socket.set_write_timeout(Some(time_left(deadline)?.max(Duration::from_millis(1))))?;
        socket
            .connect(&SockAddr::unix(&self.sock_path)?)
            .map_err(|e| match e.kind() {
                // With a send timeout set, Linux does not restart a connect
                // waiting on a full queue after a signal (a terminal resize):
                // the daemon is busy, not gone.
                std::io::ErrorKind::Interrupted => std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the connect to the daemon was interrupted while it waited",
                ),
                _ => e,
            })?;
        let stream = UnixStream::from(OwnedFd::from(socket));
        // A daemon that has already answered and closed makes these fail on
        // macOS (EINVAL); its answer is still there to read, and reading a
        // closed connection does not wait.
        let left = time_left(deadline)?;
        let _ = stream.set_read_timeout(Some(left));
        let _ = stream.set_write_timeout(Some(left));
        Ok(stream)
    }

    /// Send a minimal HTTP/1.0 request and return the body string.
    ///
    /// HTTP/1.0 is used so the server closes the connection after the
    /// response — no need to parse `Content-Length` or chunked encoding.
    /// Non-2xx responses are surfaced via [`DaemonError::BadStatus`] with
    /// the daemon's structured error envelope parsed out.
    ///
    /// The whole exchange has [`Self::REQUEST_TIMEOUT_MS`].
    fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<String, DaemonError> {
        #[cfg(unix)]
        {
            self.request_within(
                method,
                path,
                body,
                Duration::from_millis(Self::REQUEST_TIMEOUT_MS),
            )
        }
        #[cfg(not(unix))]
        {
            let _ = (method, path, body);
            Err(DaemonError::Offline {
                reason: "unix sockets unavailable on this platform".to_string(),
            })
        }
    }

    /// [`Self::request`] with `budget` for the whole exchange: the connect,
    /// every write and every read stop at one deadline.
    #[cfg(unix)]
    fn request_within(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        budget: Duration,
    ) -> Result<String, DaemonError> {
        use std::io::ErrorKind;

        let deadline = Instant::now() + budget;
        let ran_out = || DaemonError::Transport(format!("no complete answer within {budget:?}"));
        let failed = |e: std::io::Error| match e.kind() {
            ErrorKind::TimedOut | ErrorKind::WouldBlock => ran_out(),
            _ => DaemonError::Transport(e.to_string()),
        };
        let mut stream = self.connect_by(deadline).map_err(|e| match e.kind() {
            // Connected to nothing in time: a full listen queue (Linux).
            ErrorKind::TimedOut | ErrorKind::WouldBlock => ran_out(),
            _ => DaemonError::Offline {
                reason: e.to_string(),
            },
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

        // Each write and read may wait only for what is left of the budget.
        // Setting a timeout fails once the peer has shut the connection down
        // (macOS reports EINVAL). A write or read then returns at once, so
        // the timeout set before, never longer than the budget, is kept.
        let mut unsent = request.as_bytes();
        while !unsent.is_empty() {
            let _ = stream.set_write_timeout(Some(time_left(deadline).map_err(failed)?));
            match stream.write(unsent) {
                Ok(0) => {
                    return Err(DaemonError::Transport(
                        "the daemon stopped reading the request".to_string(),
                    ))
                }
                Ok(n) => unsent = &unsent[n..],
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(failed(e)),
            }
        }
        let mut raw = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            let _ = stream.set_read_timeout(Some(time_left(deadline).map_err(failed)?));
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(failed(e)),
            }
        }
        parse_response(&raw)
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

            let mut stream = match self.connect_by(Instant::now() + timeout) {
                Ok(s) => s,
                // No room in a full listen queue (Linux) within the budget:
                // the daemon is there, but busy.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
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

    /// Read familiar-memory summary rows. Read only; see [`MemorySummary`]
    /// for what is kept.
    pub fn memory_list(&self) -> Result<Vec<MemorySummary>, DaemonError> {
        parse_memory_list(&self.get("/api/v1/memory")?)
    }

    /// Read the familiar-memory overview: counts and capability state.
    pub fn memory_overview(&self) -> Result<MemoryOverview, DaemonError> {
        parse_memory_overview(&self.get("/api/v1/memory/overview")?)
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
        parse_health(&self.get("/api/v1/health")?)
    }

    /// Read the daemon's health once and say which instance answered.
    /// `timeout` bounds the whole look: connecting, sending the request and
    /// reading the answer. A peer that keeps sending bytes cannot extend it.
    ///
    /// Like [`Self::check_reachability`], only a refused connection or one
    /// that fails or closes before any byte of an answer reads as gone. A
    /// daemon that accepts and stays silent, or whose answer is cut short,
    /// is not reported as gone.
    pub fn sight(&self, timeout: Duration) -> DaemonSighting {
        #[cfg(unix)]
        {
            use std::io::ErrorKind;
            use std::os::unix::fs::MetadataExt;

            // No answer at all: gone, unless the daemon was merely slow.
            let unanswered = |e: &std::io::Error| match e.kind() {
                ErrorKind::TimedOut | ErrorKind::WouldBlock => DaemonSighting::Unidentified,
                _ => DaemonSighting::Gone,
            };

            let deadline = std::time::Instant::now() + timeout;
            let socket_file = std::fs::metadata(&self.sock_path)
                .ok()
                .map(|meta| (meta.dev(), meta.ino()));
            let mut stream = match self.connect_by(deadline) {
                Ok(stream) => stream,
                Err(e) => return unanswered(&e),
            };
            let request =
                "GET /api/v1/health HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\n\r\n";
            if let Err(e) = stream
                .write_all(request.as_bytes())
                .and_then(|()| stream.flush())
            {
                return unanswered(&e);
            }

            // Read until the daemon closes the connection, the budget runs
            // out, a read fails, or the answer is far larger than any health
            // response. Whatever arrived by then is the answer.
            const LARGEST_ANSWER: usize = 64 * 1024;
            let mut raw = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() || raw.len() > LARGEST_ANSWER {
                    break;
                }
                // Fails on macOS once the daemon has answered and closed;
                // its answer is still there to read, and that read does not
                // wait.
                let _ = stream.set_read_timeout(Some(left));
                match stream.read(&mut chunk) {
                    Ok(0) if raw.is_empty() => return DaemonSighting::Gone,
                    Ok(0) => break,
                    Ok(n) => raw.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) if raw.is_empty() => return unanswered(&e),
                    Err(_) => break,
                }
            }
            match parse_response(&raw).and_then(|body| parse_health(&body)) {
                Ok(health) if health.pid.is_some() || health.started_at.is_some() => {
                    DaemonSighting::Instance(DaemonInstance {
                        pid: health.pid,
                        started_at: health.started_at,
                        api_version: health.api_version,
                        coven_version: health.coven_version,
                        socket_file,
                    })
                }
                _ => DaemonSighting::Unidentified,
            }
        }
        #[cfg(not(unix))]
        {
            let _ = timeout;
            DaemonSighting::Gone
        }
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

    /// Register an already-running session in the daemon ledger. Best-effort.
    pub fn register_external_session(
        &self,
        req: &RegisterExternalSession,
    ) -> Result<(), DaemonError> {
        let body = serde_json::to_string(req)
            .map_err(|e| DaemonError::MalformedResponse(format!("encode request: {e}")))?;
        self.request("POST", "/api/v1/sessions/external", Some(&body))
            .map(|_| ())
    }

    /// Mark an externally-owned session finished. Best-effort.
    pub fn complete_session(
        &self,
        session_id: &str,
        exit_code: Option<i32>,
    ) -> Result<(), DaemonError> {
        let body = serde_json::to_string(&serde_json::json!({ "exitCode": exit_code }))
            .map_err(|e| DaemonError::MalformedResponse(format!("encode request: {e}")))?;
        let path = format!("/api/v1/sessions/{}/complete", url_quote(session_id));
        self.request("POST", &path, Some(&body)).map(|_| ())
    }
}

/// What is left of a request's budget before `deadline`, or a timed-out
/// error once nothing is.
#[cfg(unix)]
fn time_left(deadline: Instant) -> std::io::Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "the daemon request ran out of time",
        ))
    } else {
        Ok(left)
    }
}

/// Split a raw HTTP/1.0 response into its body, or the daemon's error.
#[cfg(unix)]
fn parse_response(raw: &[u8]) -> Result<String, DaemonError> {
    let response = String::from_utf8_lossy(raw);
    let idx = response.find("\r\n\r\n").ok_or_else(|| {
        DaemonError::MalformedResponse("missing header/body delimiter".to_string())
    })?;

    let status_line = response.lines().next().unwrap_or("");
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| {
            DaemonError::MalformedResponse(format!("could not parse status from `{status_line}`"))
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

fn parse_health(body: &str) -> Result<DaemonHealth, DaemonError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
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
// Familiar-memory reads (private parsing)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RawMemorySource {
    label: Option<String>,
}

/// `path` is deliberately absent: it is never read into the terminal.
#[derive(Deserialize)]
struct RawMemorySummary {
    id: String,
    familiar_id: String,
    title: Option<String>,
    updated_at_iso: Option<String>,
    excerpt: Option<String>,
    source: Option<RawMemorySource>,
    privacy_classification: Option<RawClassification>,
    reveal_required: Option<bool>,
    verification_state: Option<String>,
}

/// A classification label, or a shape this client does not know, which is
/// treated as unclassified.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawClassification {
    Label(String),
    Unknown(serde::de::IgnoredAny),
}

fn parse_memory_list(body: &str) -> Result<Vec<MemorySummary>, DaemonError> {
    let raw: Vec<RawMemorySummary> =
        serde_json::from_str(body).map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
    // The v1 id is an opaque UUID. Older daemons derived it from the file
    // name, so it would show what the preview withholds: refuse the shape.
    if raw
        .iter()
        .any(|row| uuid::Uuid::parse_str(&row.id).is_err())
    {
        return Err(DaemonError::UnsupportedShape("memory row id"));
    }
    Ok(raw
        .into_iter()
        .map(|row| {
            // Fail closed, as Coven Memory does in src/features/memory/privacy.ts
            // at eb542a5: only exactly "public" together with an explicit "no
            // reveal required" lets text through. Private, unknown and future
            // labels are withheld.
            let public = matches!(
                &row.privacy_classification,
                Some(RawClassification::Label(class)) if class == "public"
            );
            let preview = if public && row.reveal_required == Some(false) {
                MemoryPreview::Allowed {
                    title: row.title.unwrap_or_default(),
                    excerpt: row.excerpt.unwrap_or_default(),
                }
            } else {
                MemoryPreview::Withheld
            };
            MemorySummary {
                id: row.id,
                familiar_id: row.familiar_id,
                updated_at: row.updated_at_iso,
                source_label: row.source.and_then(|source| source.label),
                verification_state: row
                    .verification_state
                    .unwrap_or_else(|| "unknown".to_string()),
                preview,
            }
        })
        .collect())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawMemoryTotals {
    entries: Option<u64>,
    familiars: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawMemoryCapabilities {
    detail: bool,
    verification: bool,
    attestation_metadata: bool,
    supersession_history: bool,
    mutations: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawMemoryVerification {
    state: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawMemoryOverview {
    totals: RawMemoryTotals,
    last_updated_at: Option<String>,
    capabilities: RawMemoryCapabilities,
    verification: RawMemoryVerification,
}

fn parse_memory_overview(body: &str) -> Result<MemoryOverview, DaemonError> {
    let raw: RawMemoryOverview =
        serde_json::from_str(body).map_err(|e| DaemonError::MalformedResponse(e.to_string()))?;
    Ok(MemoryOverview {
        entries: raw.totals.entries,
        familiars: raw.totals.familiars,
        last_updated_at: raw.last_updated_at,
        detail: raw.capabilities.detail,
        verification: raw.capabilities.verification,
        attestation_metadata: raw.capabilities.attestation_metadata,
        supersession_history: raw.capabilities.supersession_history,
        mutations: raw.capabilities.mutations,
        verification_state: raw.verification.state,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A scripted daemon on a Unix socket, for this crate's tests.
#[cfg(all(test, unix))]
pub(crate) mod fake {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Reads each request in full, then writes what `respond` returns for
    /// it, until dropped. Dropping it leaves the socket file behind, as a
    /// killed daemon does.
    pub(crate) struct FakeDaemon {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl FakeDaemon {
        pub(crate) fn start(
            sock: &std::path::Path,
            respond: impl Fn(&str) -> String + Send + 'static,
        ) -> Self {
            let listener = std::os::unix::net::UnixListener::bind(sock).unwrap();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let thread = {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        let Ok((mut stream, _)) = listener.accept() else {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        };
                        // Accepted streams inherit O_NONBLOCK on macOS.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let request = read_request(&mut stream);
                        let _ = stream.write_all(respond(&request).as_bytes());
                    }
                })
            };
            Self {
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for FakeDaemon {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// The request's head and, when it declares one, its whole body.
    /// Answering before the request has arrived could close the stream
    /// under the client's write.
    fn read_request(stream: &mut std::os::unix::net::UnixStream) -> String {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let text = String::from_utf8_lossy(&request);
            if let Some(head_end) = text.find("\r\n\r\n") {
                let body_len = text[..head_end]
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|len| len.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= head_end + 4 + body_len {
                    break;
                }
            }
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => request.extend_from_slice(&chunk[..n]),
            }
        }
        String::from_utf8_lossy(&request).into_owned()
    }

    /// A health answer in the shape Coven v0.5.0 sends.
    pub(crate) fn health_response(pid: u32, started_at: &str) -> String {
        format!(
            "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{{\"ok\":true,\"apiVersion\":\"coven.daemon.v1\",\"covenVersion\":\"0.5.0\",\"capabilities\":{{\"eventCursor\":\"sequence\"}},\"daemon\":{{\"pid\":{pid},\"startedAt\":\"{started_at}\",\"socket\":\"/x/coven.sock\"}}}}"
        )
    }
}

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

    /// The live set is the daemon contract's nonterminal statuses; every
    /// terminal status, and one the contract does not list, is not live.
    #[test]
    fn only_nonterminal_session_statuses_are_live() {
        let session = |status: &str| DaemonSession {
            id: "s".to_string(),
            harness: "coven-code".to_string(),
            title: String::new(),
            status: status.to_string(),
            project_root: String::new(),
            archived_at: None,
        };
        for status in ["created", "running", "idle"] {
            assert!(session(status).is_live(), "{status} should be live");
        }
        for status in [
            "completed",
            "failed",
            "killed",
            "orphaned",
            "active",
            "unknown",
            "",
        ] {
            assert!(!session(status).is_live(), "{status} should not be live");
        }
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

    #[test]
    fn register_external_session_serializes_camel_case() {
        let req = RegisterExternalSession {
            id: "sess-1".to_string(),
            project_root: "/home/user/repo".to_string(),
            harness: "coven-code".to_string(),
            title: "My session".to_string(),
            transcript_path: Some("/home/user/.coven-code/sessions/sess-1.jsonl".to_string()),
            labels: vec!["source:psyche-build".to_string()],
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "sess-1",
                "projectRoot": "/home/user/repo",
                "harness": "coven-code",
                "title": "My session",
                "transcriptPath": "/home/user/.coven-code/sessions/sess-1.jsonl",
                "labels": ["source:psyche-build"],
            })
        );
    }

    #[test]
    fn register_external_session_omits_empty_labels() {
        let req = RegisterExternalSession {
            id: "sess-1".to_string(),
            project_root: "/home/user/repo".to_string(),
            harness: "coven-code".to_string(),
            title: "My session".to_string(),
            transcript_path: None,
            labels: Vec::new(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json.get("labels"), None);
    }

    #[test]
    fn complete_session_body_serializes_exit_code() {
        // Verify the exitCode key (camelCase) is what the daemon expects.
        let body = serde_json::to_string(&serde_json::json!({ "exitCode": 0i32 })).unwrap();
        assert!(
            body.contains("\"exitCode\""),
            "expected camelCase exitCode, got: {body}"
        );
        assert!(!body.contains("\"exit_code\""), "snake_case leaked: {body}");
    }

    const MEMORY_CANARY: &str = "CANARY-7f3a-private";

    fn memory_row(classification: &str, reveal_required: &str) -> String {
        format!(
            r#"{{"id":"d251bc66-3e45-5d03-8d78-1e76919642f9","familiar_id":"sage","title":"{MEMORY_CANARY} title","path":"sage/{MEMORY_CANARY}.md","updated_at":"4m ago","updated_at_iso":"2026-07-26T09:56:00Z","excerpt":"{MEMORY_CANARY} excerpt","source":{{"kind":"coven-origin","label":"Coven origin"}},"privacy_classification":{classification},"reveal_required":{reveal_required},"verification_state":"unknown"}}"#
        )
    }

    #[test]
    fn memory_not_public_or_needing_reveal_keeps_no_title_or_excerpt() {
        // Coven Memory requires a reveal unless the classification is exactly
        // "public" and reveal_required is false. Every other label, including
        // private, unknown, future, padded or differently cased ones, fails closed.
        for (classification, reveal) in [
            ("null", "null"),
            ("null", "false"),
            ("\"private\"", "true"),
            ("\"private\"", "null"),
            ("\"private\"", "false"),
            ("\"unknown\"", "false"),
            ("\"unclassified\"", "false"),
            ("\"future-label\"", "false"),
            ("\" public \"", "false"),
            ("\"Public\"", "false"),
            ("\"public\"", "true"),
            ("\"public\"", "null"),
            ("\"\"", "false"),
        ] {
            let body = format!("[{}]", memory_row(classification, reveal));
            let rows = parse_memory_list(&body).unwrap();
            assert_eq!(rows.len(), 1, "{classification} {reveal}");
            assert_eq!(
                rows[0].preview,
                MemoryPreview::Withheld,
                "{classification} {reveal}"
            );
            let held = format!("{:?}", rows[0]);
            assert!(!held.contains(MEMORY_CANARY), "{held}");
        }
    }

    #[test]
    fn a_classified_entry_without_reveal_keeps_its_preview_but_never_the_path() {
        let body = format!("[{}]", memory_row("\"public\"", "false"));
        let rows = parse_memory_list(&body).unwrap();
        let row = &rows[0];
        assert_eq!(row.id, "d251bc66-3e45-5d03-8d78-1e76919642f9");
        assert_eq!(row.familiar_id, "sage");
        assert_eq!(row.updated_at.as_deref(), Some("2026-07-26T09:56:00Z"));
        assert_eq!(row.source_label.as_deref(), Some("Coven origin"));
        assert_eq!(row.verification_state, "unknown");
        assert!(
            matches!(&row.preview, MemoryPreview::Allowed { title, .. } if title.ends_with(" title"))
        );
        assert!(!format!("{row:?}").contains(".md"), "path kept: {row:?}");
    }

    #[test]
    fn an_older_daemon_row_with_a_file_derived_id_is_unsupported_and_leaks_nothing() {
        // Daemons before the v1 dashboard reads used `{familiar}-{file stem}`
        // as the id and sent no privacy fields.
        let body = format!(
            r#"[{{"id":"sage-{MEMORY_CANARY}","familiar_id":"sage","title":"{MEMORY_CANARY}","path":"sage/{MEMORY_CANARY}.md","updated_at":"4m ago","excerpt":"{MEMORY_CANARY}"}}]"#
        );
        let error = parse_memory_list(&body).unwrap_err();
        assert!(
            matches!(error, DaemonError::UnsupportedShape(_)),
            "{error:?}"
        );
        assert!(!format!("{error:?} {error}").contains(MEMORY_CANARY));
    }

    #[test]
    fn a_structured_privacy_classification_is_withheld_not_a_failure() {
        let body = format!("[{}]", memory_row(r#"{"level":"public"}"#, "false"));
        let rows = parse_memory_list(&body).unwrap();
        assert_eq!(rows[0].preview, MemoryPreview::Withheld);
    }

    #[test]
    fn a_memory_row_without_an_id_fails_the_read() {
        let body = r#"[{"familiar_id":"sage","verification_state":"unknown"}]"#;
        assert!(parse_memory_list(body).is_err());
    }

    #[test]
    fn the_memory_overview_keeps_missing_counts_unknown_and_capabilities_off() {
        let full = r#"{"generated_at":"2026-07-26T10:00:00Z","totals":{"entries":3,"familiars":2,"verified":0,"needs_review":0,"unknown":3},"last_updated_at":"2026-07-26T09:56:00Z","capabilities":{"detail":true,"verification":false,"attestation_metadata":false,"supersession_history":false,"mutations":false},"verification":{"state":"unavailable"}}"#;
        let overview = parse_memory_overview(full).unwrap();
        assert_eq!(overview.entries, Some(3));
        assert_eq!(overview.familiars, Some(2));
        assert!(overview.detail && !overview.mutations && !overview.verification);
        assert_eq!(overview.verification_state.as_deref(), Some("unavailable"));

        let sparse = parse_memory_overview(r#"{"capabilities":{}}"#).unwrap();
        assert_eq!(sparse.entries, None);
        assert!(!sparse.detail && !sparse.mutations);
    }

    #[cfg(unix)]
    fn serve_once(
        response: &'static str,
    ) -> (
        tempfile::TempDir,
        DaemonClient,
        std::thread::JoinHandle<String>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (dir, DaemonClient { sock_path: sock }, server)
    }

    #[cfg(unix)]
    #[test]
    fn memory_list_is_a_get_and_withholds_unclassified_text() {
        let (_dir, client, server) = serve_once(
            "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n[{\"id\":\"d251bc66-3e45-5d03-8d78-1e76919642f9\",\"familiar_id\":\"sage\",\"title\":\"CANARY-7f3a-private\",\"excerpt\":\"CANARY-7f3a-private\",\"privacy_classification\":null,\"reveal_required\":null,\"verification_state\":\"unknown\"}]",
        );
        let rows = client.memory_list().unwrap();
        let request = server.join().unwrap();
        assert!(
            request.starts_with("GET /api/v1/memory HTTP/1.0"),
            "{request}"
        );
        assert_eq!(rows[0].preview, MemoryPreview::Withheld);
        assert!(!format!("{rows:?}").contains(MEMORY_CANARY));
    }

    #[cfg(unix)]
    #[test]
    fn an_older_daemon_without_memory_reads_reports_not_found() {
        let (_dir, client, server) = serve_once(
            "HTTP/1.0 404 Not Found\r\nContent-Type: application/json\r\n\r\n{\"error\":{\"code\":\"not_found\",\"message\":\"no route\"}}",
        );
        let error = client.memory_overview().unwrap_err();
        let request = server.join().unwrap();
        assert!(
            request.starts_with("GET /api/v1/memory/overview HTTP/1.0"),
            "{request}"
        );
        assert_eq!(error.status(), Some(404));
    }

    #[cfg(unix)]
    use super::fake::{health_response, FakeDaemon};

    #[cfg(unix)]
    const SIGHT_BUDGET: Duration = Duration::from_secs(5);

    /// #412, policy B: the epoch is what health says about the serving
    /// process plus the socket file. One daemon gives the same instance on
    /// every sighting.
    #[cfg(unix)]
    #[test]
    fn sightings_of_one_daemon_name_the_same_instance() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let _daemon =
            FakeDaemon::start(&sock, |_| health_response(4242, "2026-11-18T05:06:40.214Z"));
        let client = DaemonClient {
            sock_path: sock.clone(),
        };

        let first = client.sight(SIGHT_BUDGET);
        let meta = fs::metadata(&sock).unwrap();
        assert_eq!(
            first,
            DaemonSighting::Instance(DaemonInstance {
                pid: Some(4242),
                started_at: Some("2026-11-18T05:06:40.214Z".to_string()),
                api_version: "coven.daemon.v1".to_string(),
                coven_version: "0.5.0".to_string(),
                socket_file: Some((meta.dev(), meta.ino())),
            })
        );
        assert_eq!(client.sight(SIGHT_BUDGET), first);
    }

    /// A daemon restarted on the same socket path is a different instance,
    /// whether its pid, its start time or both changed, and a dead daemon
    /// that left its socket file behind is gone, not unidentified.
    #[cfg(unix)]
    #[test]
    fn a_restarted_daemon_is_a_different_instance() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let client = DaemonClient {
            sock_path: sock.clone(),
        };
        let sight_of = |pid: u32, started_at: &str| {
            let response = health_response(pid, started_at);
            let daemon = FakeDaemon::start(&sock, move |_| response.clone());
            let sighting = client.sight(SIGHT_BUDGET);
            drop(daemon);
            assert!(sock.exists(), "the stale socket file stays");
            assert_eq!(client.sight(SIGHT_BUDGET), DaemonSighting::Gone);
            fs::remove_file(&sock).unwrap();
            assert_eq!(client.sight(SIGHT_BUDGET), DaemonSighting::Gone);
            sighting
        };

        let first = sight_of(100, "2026-11-18T05:00:00Z");
        let new_start = sight_of(100, "2026-11-18T05:00:09Z");
        let new_pid = sight_of(101, "2026-11-18T05:00:00Z");
        let both = sight_of(102, "2026-11-18T05:00:20Z");
        // Each fake binds a new socket file. Compare what the daemon says
        // about itself alone, so the file's identity cannot carry the test.
        let named = |sighting: &DaemonSighting| match sighting {
            DaemonSighting::Instance(instance) => DaemonInstance {
                socket_file: None,
                ..instance.clone()
            },
            other => panic!("not an instance: {other:?}"),
        };
        for (name, later) in [
            ("start time", &new_start),
            ("pid", &new_pid),
            ("both", &both),
        ] {
            assert_ne!(
                named(&first),
                named(later),
                "a new {name} must read as a new instance"
            );
        }
    }

    /// A daemon that accepts and never answers is neither gone nor an
    /// instance, and the sighting returns within its budget.
    #[cfg(unix)]
    #[test]
    fn a_silent_daemon_is_unidentified_within_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let accepted = listener.accept();
            // Hold the connection open, silent, until the test is done.
            let _ = held.recv();
            drop(accepted);
        });

        let client = DaemonClient { sock_path: sock };
        let started = std::time::Instant::now();
        let sighting = client.sight(Duration::from_millis(300));
        let elapsed = started.elapsed();
        drop(release);
        server.join().unwrap();

        assert_eq!(sighting, DaemonSighting::Unidentified);
        assert!(
            elapsed >= Duration::from_millis(300) && elapsed < Duration::from_secs(3),
            "sight took {elapsed:?} against a 300 ms budget"
        );
    }

    /// The budget bounds the whole look, not each read: a peer that keeps
    /// sending bytes without ever finishing cannot hold the probe.
    #[cfg(unix)]
    #[test]
    fn a_daemon_that_trickles_bytes_is_unidentified_within_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // One byte every 50 ms, for far longer than the budget, until
            // the client hangs up.
            for _ in 0..200 {
                if stream.write_all(b"H").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });

        let client = DaemonClient { sock_path: sock };
        let started = std::time::Instant::now();
        let sighting = client.sight(Duration::from_millis(300));
        let elapsed = started.elapsed();
        server.join().unwrap();

        assert_eq!(sighting, DaemonSighting::Unidentified);
        assert!(
            elapsed >= Duration::from_millis(300) && elapsed < Duration::from_secs(3),
            "sight took {elapsed:?} against a 300 ms budget"
        );
    }

    /// #580: one deadline covers a whole request. A peer that sends a byte
    /// every second never lets a single read time out, so it must not be
    /// able to hold the request past the request timeout.
    #[cfg(unix)]
    #[test]
    fn a_request_to_a_trickling_daemon_ends_at_its_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            // One byte a second for 15 s, or until the client hangs up.
            for _ in 0..15 {
                if stream.write_all(b"H").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });

        let client = DaemonClient { sock_path: sock };
        let started = std::time::Instant::now();
        let result = client.health();
        let elapsed = started.elapsed();
        server.join().unwrap();

        assert!(result.is_err(), "a trickled answer never completes");
        assert!(
            elapsed < Duration::from_secs(7),
            "the request took {elapsed:?} against its 5 s deadline"
        );
    }

    /// #580: a daemon that accepts and never answers ends a request at its
    /// deadline, as a transport error that says so.
    #[cfg(unix)]
    #[test]
    fn a_request_to_a_silent_daemon_ends_at_its_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let accepted = listener.accept();
            let _ = held.recv();
            drop(accepted);
        });

        let client = DaemonClient { sock_path: sock };
        let started = std::time::Instant::now();
        let result =
            client.request_within("GET", "/api/v1/sessions", None, Duration::from_millis(300));
        let elapsed = started.elapsed();
        drop(release);
        server.join().unwrap();

        match result {
            Err(DaemonError::Transport(reason)) => {
                assert_eq!(reason, "no complete answer within 300ms")
            }
            other => panic!("expected a transport error, got {other:?}"),
        }
        assert!(
            elapsed >= Duration::from_millis(300) && elapsed < Duration::from_secs(3),
            "the request took {elapsed:?} against a 300 ms budget"
        );
    }

    /// #580: the connect has a deadline too. On Linux a connect to a socket
    /// whose listen queue is full waits for room; macOS refuses it at once.
    #[cfg(unix)]
    #[test]
    fn a_connect_to_a_full_listen_queue_ends_at_its_deadline() {
        use socket2::{Domain, SockAddr, Socket, Type};

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let address = SockAddr::unix(&sock).unwrap();
        let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
        listener.bind(&address).unwrap();
        // Room for at most one waiting connection, and nothing accepts.
        listener.listen(0).unwrap();
        let mut waiting = Vec::new();
        for _ in 0..64 {
            let filler = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
            filler.set_nonblocking(true).unwrap();
            if filler.connect(&address).is_err() {
                break;
            }
            waiting.push(filler);
        }

        let client = DaemonClient { sock_path: sock };
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = client.health();
            let _ = done.send((result.is_err(), started.elapsed()));
        });
        let (failed, elapsed) = finished
            .recv_timeout(Duration::from_secs(10))
            .expect("the request was still connecting after 10 s");
        assert!(failed, "nothing accepted the connection");
        assert!(
            elapsed < Duration::from_secs(7),
            "the request took {elapsed:?} against its 5 s deadline"
        );
        drop((listener, waiting));
    }

    /// A daemon that answers in full and then keeps the connection open is
    /// still identified, once the budget ends.
    #[cfg(unix)]
    #[test]
    fn a_complete_answer_on_a_connection_left_open_names_the_instance() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("coven.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(health_response(4242, "2026-11-18T05:06:40.214Z").as_bytes())
                .unwrap();
            // Keep the connection open until the test is done.
            let _ = held.recv();
        });

        let client = DaemonClient { sock_path: sock };
        let sighting = client.sight(Duration::from_millis(300));
        drop(release);
        server.join().unwrap();

        let DaemonSighting::Instance(instance) = sighting else {
            panic!("not an instance: {sighting:?}");
        };
        assert_eq!(instance.pid, Some(4242));
        assert_eq!(
            instance.started_at.as_deref(),
            Some("2026-11-18T05:06:40.214Z")
        );
    }

    /// What a connection that was made and then went wrong reads as: closed
    /// with no byte of an answer is gone; an answer cut short is not. And a
    /// health body that names a pid and no start time is still an instance.
    #[cfg(unix)]
    #[test]
    fn a_closed_or_cut_short_answer_and_a_pid_alone() {
        let (_dir, client, server) = serve_once("");
        assert_eq!(client.sight(SIGHT_BUDGET), DaemonSighting::Gone);
        server.join().unwrap();

        let (_dir, client, server) = serve_once(
            "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true,\"daemon\":{\"pi",
        );
        assert_eq!(client.sight(SIGHT_BUDGET), DaemonSighting::Unidentified);
        server.join().unwrap();

        let (_dir, client, server) = serve_once(
            "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true,\"apiVersion\":\"coven.daemon.v1\",\"covenVersion\":\"0.5.0\",\"daemon\":{\"pid\":77}}",
        );
        let sighting = client.sight(SIGHT_BUDGET);
        server.join().unwrap();
        let DaemonSighting::Instance(instance) = sighting else {
            panic!("not an instance: {sighting:?}");
        };
        assert_eq!((instance.pid, instance.started_at), (Some(77), None));
    }

    /// An answer that names no process cannot be told from its successor,
    /// so it is not an instance: an error status, a body without the
    /// `daemon` object, and a body that is not JSON.
    #[cfg(unix)]
    #[test]
    fn an_answer_that_names_no_process_is_unidentified() {
        for response in [
            "HTTP/1.0 404 Not Found\r\nContent-Type: application/json\r\n\r\n{\"error\":{\"code\":\"not_found\",\"message\":\"no route\"}}",
            "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true,\"apiVersion\":\"coven.daemon.v1\"}",
            "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n[]",
            "HTTP/1.0 200 OK\r\n\r\nnot json",
        ] {
            let (_dir, client, server) = serve_once(response);
            assert_eq!(
                client.sight(SIGHT_BUDGET),
                DaemonSighting::Unidentified,
                "{response}"
            );
            let request = server.join().unwrap();
            assert!(
                request.starts_with("GET /api/v1/health HTTP/1.0"),
                "{request}"
            );
        }
    }

    /// Opt-in evidence against a real daemon whose `COVEN_HOME` holds only
    /// synthetic memory containing `CANARY-7f3a-private`. Never point this at
    /// a home with real memory. Run with
    /// `COVEN_MEMORY_LIVE_TEST=1 COVEN_HOME=<disposable home> cargo test -p claurst-core --lib live_daemon -- --ignored`.
    #[test]
    #[ignore = "needs a disposable Coven daemon with synthetic memory"]
    fn live_daemon_memory_reads_withhold_unclassified_text() {
        // Refuse anything but an explicit, disposable home: the default
        // `~/.coven` holds real memory.
        assert_eq!(
            std::env::var("COVEN_MEMORY_LIVE_TEST").as_deref(),
            Ok("1"),
            "set COVEN_MEMORY_LIVE_TEST=1 and a disposable COVEN_HOME"
        );
        let home = std::env::var("COVEN_HOME").expect("COVEN_HOME must be set");
        let real = dirs::home_dir().map(|dir| dir.join(".coven"));
        assert!(
            real.as_deref() != Some(std::path::Path::new(&home)),
            "refusing the real ~/.coven"
        );
        let client = DaemonClient::new().expect("daemon socket in COVEN_HOME");
        let rows = client.memory_list().expect("memory list");
        assert!(!rows.is_empty(), "synthetic memory expected");
        // Report only counts and booleans, never row content.
        let held = format!("{rows:?}");
        assert!(!held.contains(MEMORY_CANARY), "canary kept");
        assert!(!held.contains(".md"), "path kept");
        let withheld = rows
            .iter()
            .filter(|row| row.preview == MemoryPreview::Withheld)
            .count();
        assert_eq!(
            withheld,
            rows.len(),
            "{withheld} of {} withheld",
            rows.len()
        );
        let overview = client.memory_overview().expect("memory overview");
        assert!(!overview.mutations, "mutations offered");
        eprintln!(
            "live rows={} withheld={withheld} entries={:?} familiars={:?} detail={} mutations={}",
            rows.len(),
            overview.entries,
            overview.familiars,
            overview.detail,
            overview.mutations
        );
    }
}
