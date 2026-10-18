//! Top-level ACP request / notification dispatcher.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_client_protocol_schema as acp;
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::connection::{Connection, Inbound};
use crate::runtime::AgentRuntime;
use crate::sessions::{SessionRegistry, SessionState};

/// The ACP agent: owns the connection, the runtime, and the session registry.
pub struct AgentServer {
    pub connection: Arc<Connection>,
    pub runtime: Arc<AgentRuntime>,
    pub sessions: Arc<SessionRegistry>,
    pub client_capabilities: parking_lot::RwLock<acp::ClientCapabilities>,
}

impl AgentServer {
    pub fn new(connection: Arc<Connection>, runtime: Arc<AgentRuntime>) -> Arc<Self> {
        Arc::new(Self {
            connection,
            runtime,
            sessions: Arc::new(SessionRegistry::new()),
            client_capabilities: parking_lot::RwLock::new(acp::ClientCapabilities::default()),
        })
    }

    /// Dispatch a single inbound message. Spawns the actual handler on a
    /// background task so the reader loop stays responsive while a prompt
    /// is in flight. Returns the join handle so the caller can wait for
    /// in-flight work to finish before shutting down.
    pub fn dispatch(self: &Arc<Self>, msg: Inbound) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            match msg {
                Inbound::Request { id, method, params } => {
                    let response = this.handle_request(&method, params).await;
                    let result = match response {
                        Ok(value) => this.connection.send_response(id, value).await,
                        Err(err) => this.connection.send_error_response(id, err).await,
                    };
                    if let Err(e) = result {
                        let io_kind = e.downcast_ref::<std::io::Error>().map(std::io::Error::kind);
                        warn!(
                            method = diagnostic_method(&method),
                            method_bytes = method.len(),
                            error_class = if io_kind.is_some() { "io" } else { "other" },
                            ?io_kind,
                            "ACP: failed to send response"
                        );
                    }
                }
                Inbound::Notification { method, params } => {
                    this.handle_notification(&method, params).await;
                }
            }
        })
    }

    async fn handle_request(
        self: &Arc<Self>,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, acp::Error> {
        debug!(
            method = diagnostic_method(method),
            method_bytes = method.len(),
            "ACP: dispatch request"
        );
        match method {
            "initialize" => {
                let req: acp::InitializeRequest = parse_params(params)?;
                let result = self.on_initialize(req).await?;
                serde_json::to_value(result).map_err(|_| acp::Error::internal_error())
            }
            "authenticate" => {
                let _req: acp::AuthenticateRequest = parse_params(params)?;
                // Coven Code uses local credentials; clients don't need to authenticate.
                serde_json::to_value(acp::AuthenticateResponse::default())
                    .map_err(|_| acp::Error::internal_error())
            }
            "session/new" => {
                let req: acp::NewSessionRequest = parse_params(params)?;
                let result = self.on_new_session(req).await?;
                serde_json::to_value(result).map_err(|_| acp::Error::internal_error())
            }
            "session/load" => {
                // v1: not supported. Capability is advertised as false in
                // initialize so a well-behaved client never calls this.
                Err(acp::Error::method_not_found())
            }
            "session/prompt" => {
                let req: acp::PromptRequest = parse_params(params)?;
                let result = self.on_prompt(req).await?;
                serde_json::to_value(result).map_err(|_| acp::Error::internal_error())
            }
            other => {
                warn!(
                    method = diagnostic_method(other),
                    method_bytes = other.len(),
                    "ACP: method not found"
                );
                Err(acp::Error::method_not_found())
            }
        }
    }

    async fn handle_notification(self: &Arc<Self>, method: &str, params: Option<Value>) {
        debug!(
            method = diagnostic_method(method),
            method_bytes = method.len(),
            "ACP: dispatch notification"
        );
        match method {
            "session/cancel" => {
                let parsed: Result<acp::CancelNotification, _> = params
                    .map(serde_json::from_value)
                    .unwrap_or(Err(serde::de::Error::custom("missing params")));
                match parsed {
                    Ok(notif) => {
                        if let Some(session) = self.sessions.get(&notif.session_id) {
                            info!(session_id = %notif.session_id, "ACP: cancelling session");
                            session.cancel_turn();
                        }
                    }
                    Err(e) => warn!(
                        category = ?e.classify(),
                        line = e.line(),
                        column = e.column(),
                        "ACP: malformed session/cancel notification"
                    ),
                }
            }
            other => {
                warn!(
                    method = diagnostic_method(other),
                    method_bytes = other.len(),
                    "ACP: ignoring unknown notification"
                );
            }
        }
    }

    async fn on_initialize(
        self: &Arc<Self>,
        req: acp::InitializeRequest,
    ) -> Result<acp::InitializeResponse, acp::Error> {
        info!(
            client_version = ?req.client_info.as_ref().map(|i| (&i.name, &i.version)),
            "ACP: initialize"
        );
        *self.client_capabilities.write() = req.client_capabilities.clone();

        let agent_info = acp::Implementation::new("coven-code", env!("CARGO_PKG_VERSION"))
            .title(Some("Coven".to_string()));

        let mut response = acp::InitializeResponse::new(acp::ProtocolVersion::V1)
            .agent_capabilities(
                acp::AgentCapabilities::new()
                    .load_session(false)
                    .prompt_capabilities(acp::PromptCapabilities::new())
                    .mcp_capabilities(acp::McpCapabilities::new()),
            );
        response = response.agent_info(Some(agent_info));
        Ok(response)
    }

    async fn on_new_session(
        self: &Arc<Self>,
        req: acp::NewSessionRequest,
    ) -> Result<acp::NewSessionResponse, acp::Error> {
        let cwd = validate_session_cwd(&req.cwd)?;
        let session_id = acp::SessionId::new(format!("acp-{}", uuid::Uuid::new_v4()));
        let state = SessionState::new(session_id.clone(), cwd.clone());
        info!(session_id = %session_id, cwd = ?cwd, "ACP: new session");

        // v1: ignore req.mcp_servers — agent uses settings.json MCP roster.
        if !req.mcp_servers.is_empty() {
            warn!(
                count = req.mcp_servers.len(),
                "ACP: session-specific MCP servers are not yet routed (v1) — using global config"
            );
        }

        self.sessions.insert(state);
        Ok(acp::NewSessionResponse::new(session_id))
    }

    async fn on_prompt(
        self: &Arc<Self>,
        req: acp::PromptRequest,
    ) -> Result<acp::PromptResponse, acp::Error> {
        let session = match self.sessions.get(&req.session_id) {
            Some(s) => s,
            None => {
                return Err(acp::Error::invalid_params().data(Some(serde_json::json!({
                    "reason": "unknown session",
                    "sessionId": req.session_id,
                }))));
            }
        };
        crate::prompt::handle(self.runtime.clone(), self.connection.clone(), session, req).await
    }
}

fn validate_session_cwd(cwd: &Path) -> Result<PathBuf, acp::Error> {
    let invalid = |reason| {
        acp::Error::invalid_params().data(Some(serde_json::json!({
            "reason": reason,
        })))
    };
    if !cwd.is_absolute() {
        return Err(invalid("cwd must be absolute"));
    }
    let canonical =
        std::fs::canonicalize(cwd).map_err(|_| invalid("cwd must be an existing directory"))?;
    if !canonical.is_dir() {
        return Err(invalid("cwd must be an existing directory"));
    }
    let display = canonical
        .to_str()
        .ok_or_else(|| invalid("cwd must be UTF-8"))?;
    if display.chars().any(char::is_control) {
        return Err(invalid("cwd must not contain control characters"));
    }
    Ok(canonical)
}

// Never return client-provided strings for diagnostics, even at DEBUG level.
fn diagnostic_method(method: &str) -> &'static str {
    match method {
        "initialize" => "initialize",
        "authenticate" => "authenticate",
        "session/new" => "session/new",
        "session/load" => "session/load",
        "session/prompt" => "session/prompt",
        "session/cancel" => "session/cancel",
        _ => "unknown",
    }
}

fn parse_params<T: serde::de::DeserializeOwned>(params: Option<Value>) -> Result<T, acp::Error> {
    let value = params.ok_or_else(acp::Error::invalid_params)?;
    serde_json::from_value(value).map_err(|e| {
        acp::Error::invalid_params().data(Some(
            serde_json::json!({ "deserialize_error": e.to_string() }),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cwd_must_resolve_to_a_display_safe_directory() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("project");
        std::fs::create_dir(&directory).unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, b"fixture").unwrap();

        assert_eq!(
            validate_session_cwd(&directory.join("..")).unwrap(),
            root.path().canonicalize().unwrap()
        );
        for (cwd, reason) in [
            (PathBuf::from("relative"), "cwd must be absolute"),
            (
                root.path().join("missing"),
                "cwd must be an existing directory",
            ),
            (file, "cwd must be an existing directory"),
        ] {
            let error = validate_session_cwd(&cwd).unwrap_err();
            assert_eq!(error.data, Some(serde_json::json!({ "reason": reason })));
        }

        #[cfg(unix)]
        {
            let newline = root.path().join("line\nbreak");
            std::fs::create_dir(&newline).unwrap();
            let error = validate_session_cwd(&newline).unwrap_err();
            assert_eq!(
                error.data,
                Some(serde_json::json!({ "reason": "cwd must not contain control characters" }))
            );
        }
    }
}
