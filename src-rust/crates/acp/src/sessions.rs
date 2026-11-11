//! Per-session state for the ACP server.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol_schema as acp;
use claurst_core::types::Message;
use dashmap::DashMap;
use tokio_util::sync::CancellationToken;

/// One ACP session — a logical conversation with its own cwd, transcript,
/// MCP server roster, and cancellation token.
pub struct SessionState {
    pub session_id: acp::SessionId,
    pub cwd: PathBuf,
    pub messages: parking_lot::Mutex<Vec<Message>>,
    pub cancel_token: parking_lot::Mutex<CancellationToken>,
    pub next_permission_id: Arc<std::sync::atomic::AtomicU64>,
    pub file_history: Arc<parking_lot::Mutex<claurst_core::file_history::FileHistory>>,
    pub current_turn: Arc<std::sync::atomic::AtomicUsize>,
}

impl SessionState {
    /// Cancel the in-flight turn without cancelling subsequent prompts.
    pub fn cancel_turn(&self) {
        let mut token = self.cancel_token.lock();
        token.cancel();
        *token = CancellationToken::new();
    }

    pub fn new(session_id: acp::SessionId, cwd: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            session_id,
            cwd,
            messages: parking_lot::Mutex::new(Vec::new()),
            cancel_token: parking_lot::Mutex::new(CancellationToken::new()),
            next_permission_id: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            file_history: Arc::new(parking_lot::Mutex::new(
                claurst_core::file_history::FileHistory::new(),
            )),
            current_turn: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }
}

/// Map of active sessions keyed by ACP session id.
#[derive(Default)]
pub struct SessionRegistry {
    inner: DashMap<acp::SessionId, Arc<SessionState>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, state: Arc<SessionState>) {
        self.inner.insert(state.session_id.clone(), state);
    }

    pub fn get(&self, id: &acp::SessionId) -> Option<Arc<SessionState>> {
        self.inner.get(id).map(|r| r.value().clone())
    }

    pub fn remove(&self, id: &acp::SessionId) -> Option<Arc<SessionState>> {
        self.inner.remove(id).map(|(_, v)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_only_affects_current_turns() {
        let session = SessionState::new(acp::SessionId::new("test"), PathBuf::from("/tmp"));
        let first = session.cancel_token.lock().clone();
        let concurrent = session.cancel_token.lock().clone();
        session.cancel_turn();
        assert!(first.is_cancelled());
        assert!(concurrent.is_cancelled());
        let next = session.cancel_token.lock().clone();
        assert!(
            !next.is_cancelled(),
            "next prompt inherits previous cancellation"
        );
        session.cancel_turn();
        assert!(next.is_cancelled());
        assert!(!session.cancel_token.lock().is_cancelled());
    }

    #[test]
    fn idle_cancellation_does_not_poison_next_prompt() {
        let session = SessionState::new(acp::SessionId::new("test"), PathBuf::from("/tmp"));
        session.cancel_turn();
        session.cancel_turn();
        assert!(!session.cancel_token.lock().is_cancelled());
    }
}
