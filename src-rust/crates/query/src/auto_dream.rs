//! AutoDream: automatic memory consolidation daemon
//!
//! Background memory consolidation. Fires a consolidation prompt as a forked
//! subagent when the time gate passes AND enough sessions have accumulated.
//!
//! Gate order (cheapest first):
//!   1. Time:     hours since last_consolidated_at >= min_hours  (one stat)
//!   2. Sessions: transcript count with mtime > last_consolidated_at >= min_sessions
//!   3. Lock:     no other process holds the consolidation file lock

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;

// Scan throttle: when time-gate passes but session-gate doesn't, the lock
// mtime doesn't advance, so the time-gate keeps passing every turn.
pub const SESSION_SCAN_INTERVAL_SECS: u64 = 10 * 60; // 10 minutes

/// GrowthBook-sourced scheduling config (with defaults)
#[derive(Debug, Clone)]
pub struct AutoDreamConfig {
    /// Minimum hours between consolidations (default: 24)
    pub min_hours: f64,
    /// Minimum new-session count to trigger (default: 5)
    pub min_sessions: usize,
}

impl Default for AutoDreamConfig {
    fn default() -> Self {
        Self {
            min_hours: 24.0,
            min_sessions: 5,
        }
    }
}

/// Persisted state written to `.consolidation_state.json`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConsolidationState {
    /// Unix timestamp (seconds) of last successful consolidation.
    /// `None` means never consolidated.
    pub last_consolidated_at: Option<u64>,
    /// ETag / opaque lock token – reserved for future distributed locking.
    pub lock_etag: Option<String>,
}

/// Data returned by `AutoDream::maybe_trigger` when consolidation should proceed.
/// Pass this to `AutoDream::finish_consolidation` after the agent completes.
#[derive(Debug, Clone)]
pub struct ConsolidationTask {
    /// The consolidation prompt to send to the sub-agent.
    pub prompt: String,
    /// Working directory for the sub-agent.
    pub memory_dir: PathBuf,
    /// Path to the state file (written after consolidation completes).
    pub state_file: PathBuf,
    /// Persistent lock path; never unlink a potentially locked inode.
    pub lock_file: PathBuf,
    /// Ownership lasts until every execution/finalization lease is dropped.
    pub(crate) lock: Arc<std::fs::File>,
}

/// Core AutoDream logic; owns path state, delegates I/O to async methods.
pub struct AutoDream {
    config: AutoDreamConfig,
    memory_dir: PathBuf,
    conversations_dir: PathBuf,
    lock_file: PathBuf,
    state_file: PathBuf,
}

impl AutoDream {
    pub fn new(memory_dir: PathBuf, conversations_dir: PathBuf) -> Self {
        let lock_file = memory_dir.join(".consolidation_lock");
        let state_file = memory_dir.join(".consolidation_state.json");
        Self {
            config: AutoDreamConfig::default(),
            memory_dir,
            conversations_dir,
            lock_file,
            state_file,
        }
    }

    /// Construct with explicit config (for testing / feature-flag overrides).
    pub fn with_config(
        config: AutoDreamConfig,
        memory_dir: PathBuf,
        conversations_dir: PathBuf,
    ) -> Self {
        let lock_file = memory_dir.join(".consolidation_lock");
        let state_file = memory_dir.join(".consolidation_state.json");
        Self {
            config,
            memory_dir,
            conversations_dir,
            lock_file,
            state_file,
        }
    }

    // -------------------------------------------------------------------------
    // Gate checks
    // -------------------------------------------------------------------------

    /// Check all gates cheapest-first.  Returns `true` if consolidation should run.
    pub async fn should_consolidate(&self, state: &ConsolidationState) -> Result<bool> {
        // Gate 1: Time gate (cheapest – one arithmetic check)
        if !self.time_gate_passes(state) {
            return Ok(false);
        }

        // Gate 2: Session gate (directory scan)
        if !self.session_gate_passes(state).await? {
            return Ok(false);
        }

        // Gate 3: Lock gate (no other process mid-consolidation)
        if !self.lock_gate_passes().await? {
            return Ok(false);
        }

        Ok(true)
    }

    fn time_gate_passes(&self, state: &ConsolidationState) -> bool {
        let now_secs = now_secs();
        match state.last_consolidated_at {
            None => true, // Never consolidated → always pass
            Some(last) => {
                let hours_elapsed = (now_secs.saturating_sub(last)) as f64 / 3600.0;
                hours_elapsed >= self.config.min_hours
            }
        }
    }

    async fn session_gate_passes(&self, state: &ConsolidationState) -> Result<bool> {
        let last_secs = state.last_consolidated_at.unwrap_or(0);
        let mut count = 0usize;

        if !self.conversations_dir.exists() {
            return Ok(false);
        }

        let mut dir = fs::read_dir(&self.conversations_dir).await?;
        while let Some(entry) = dir.next_entry().await? {
            let metadata = entry.metadata().await?;
            if let Ok(mtime) = metadata.modified() {
                let mtime_secs = mtime
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_secs();
                if mtime_secs > last_secs {
                    count += 1;
                    if count >= self.config.min_sessions {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    async fn lock_gate_passes(&self) -> Result<bool> {
        match self.acquire_lock().await {
            Ok(_lease) => Ok(true),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::WouldBlock) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    /// Acquire exclusive ownership without truncating or unlinking the lock file.
    /// Dropping the last lease releases the OS lock, including after process exit.
    pub async fn acquire_lock(&self) -> Result<Arc<std::fs::File>> {
        if let Some(parent) = self.lock_file.parent() {
            fs::create_dir_all(parent).await?;
        }
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock_file)?;
        file.try_lock().map_err(std::io::Error::from)?;
        Ok(Arc::new(file))
    }

    // -------------------------------------------------------------------------
    // State persistence
    // -------------------------------------------------------------------------

    /// Stamp `last_consolidated_at = now` and persist.
    pub async fn update_state(&self, state: &mut ConsolidationState) -> Result<()> {
        state.last_consolidated_at = Some(now_secs());
        let json = serde_json::to_string_pretty(state)?;
        if let Some(parent) = self.state_file.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&self.state_file, json).await?;
        Ok(())
    }

    /// Load persisted state; returns `Default` on any error (missing file, parse failure).
    pub async fn load_state(&self) -> ConsolidationState {
        match fs::read_to_string(&self.state_file).await {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => ConsolidationState::default(),
        }
    }

    // -------------------------------------------------------------------------
    // High-level trigger
    // -------------------------------------------------------------------------

    /// Check all gates and, if they pass, acquire the lock and return the info
    /// needed to run the consolidation subagent.
    ///
    /// Returns `Ok(Some(task))` if consolidation should proceed (lock acquired),
    /// `Ok(None)` if gated out, or `Err` for hard I/O failures.
    ///
    /// The caller is responsible for actually running the agent and calling
    /// `finish_consolidation` when done.
    pub async fn maybe_trigger(&self) -> Result<Option<ConsolidationTask>> {
        let state = self.load_state().await;
        if !self.should_consolidate(&state).await? {
            return Ok(None);
        }
        let lock = match self.acquire_lock().await {
            Ok(lock) => lock,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::WouldBlock) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        // A previous owner may have persisted completion while we were checking
        // the cheap gates. Re-read after acquiring exclusive ownership.
        let state = self.load_state().await;
        if !self.time_gate_passes(&state) || !self.session_gate_passes(&state).await? {
            return Ok(None);
        }
        Ok(Some(ConsolidationTask {
            prompt: self.consolidation_prompt(),
            memory_dir: self.memory_dir.clone(),
            state_file: self.state_file.clone(),
            lock_file: self.lock_file.clone(),
            lock,
        }))
    }

    /// Persist success while the caller still owns exclusion. Errors retain the
    /// task's lease so its owner can retry without admitting another execution.
    pub async fn finish_consolidation(task: &ConsolidationTask) -> Result<()> {
        let updated = ConsolidationState {
            last_consolidated_at: Some(now_secs()),
            lock_etag: None,
        };
        let json = serde_json::to_string_pretty(&updated)?;
        // Keep the tiny state write in this poll: cancelling an async fs write
        // can otherwise leave its blocking worker writing after the lease drops.
        std::fs::write(&task.state_file, json)?;
        Ok(())
    }

    /// Wait for actual query exit and the owned cleanup attempt. Monitor status
    /// is not an exit acknowledgement, and partial output is not success.
    pub(crate) async fn finish_after_execution(
        task: ConsolidationTask,
        mut completed: tokio::sync::oneshot::Receiver<crate::agent_tool::AgentCompletion>,
        cancel: tokio_util::sync::CancellationToken,
        goal: claurst_core::goal::GoalTurnContext,
        session_id: String,
    ) -> Result<()> {
        let mut revoked = false;
        let outcome = loop {
            if goal.require_work(&session_id).is_err() {
                revoked = true;
                cancel.cancel();
            }
            tokio::select! {
                result = &mut completed => break result.map_err(|_| {
                    anyhow::anyhow!("AutoDream execution ended without a completion acknowledgement")
                })?,
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
        };
        if outcome != crate::agent_tool::AgentCompletion::Succeeded || revoked {
            return Ok(());
        }
        loop {
            if cancel.is_cancelled() || goal.require_work(&session_id).is_err() {
                return Ok(());
            }
            match Self::finish_consolidation(&task).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    // Keep actual ownership on persistence failure, not merely a
                    // leftover pathname. Shutdown drops this owner; revocation
                    // releases it without recording success on the next check.
                    tracing::warn!(%error, "AutoDream state persistence failed; retaining lock for retry");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Prompt construction
    // -------------------------------------------------------------------------

    /// Build the consolidation prompt for the forked subagent.
    pub fn consolidation_prompt(&self) -> String {
        format!(
            r#"# Dream: Memory Consolidation

You are performing a dream — a reflective pass over your memory files. Synthesize what you have learned recently into durable, well-organized memories so that future sessions can orient quickly.

Memory directory: `{memory_dir}`

Session transcripts are intentionally unavailable to this automatic background task. Do not inspect historical conversation JSONL files or any other transcript store.

---

## Phase 1 — Orient

- List the memory directory to see what already exists
- Read `MEMORY.md` to understand the current index
- Skim existing topic files so you improve them rather than creating duplicates

## Phase 2 — Gather recent signal

Look for new information worth persisting from existing memory files only:

1. **Daily logs** (`logs/YYYY/MM/YYYY-MM-DD.md`) if present
2. **Existing memories that drifted** — facts that contradict what you see now

Do not inspect session transcripts, conversation logs, repository files, web pages, or other external data sources.

## Phase 3 — Consolidate

For each thing worth remembering, draft the exact memory-file changes that should be applied later with foreground user approval. Focus on:
- Merging new signal into existing topic files rather than creating near-duplicates
- Converting relative dates to absolute dates
- Identifying contradicted facts

## Phase 4 — Prune and index

Review `MEMORY.md` for changes that would keep it under 200 lines and ~25 KB. It is an **index**, not a dump.
Each entry: `- [Title](file.md) — one-line hook`

- Identify pointers to stale, wrong, or superseded memories
- Suggest shorter entries where entries are verbose
- Suggest pointers to newly important memories
- Identify contradictions

---

Return a brief summary and any suggested file changes. If nothing should change, say so.

**Tool constraints for this run:** This automatic background task is read-only. Use only the provided read-only file discovery tools. Anything that writes, runs shell commands, accesses the network, or reads transcripts will be denied.
"#,
            memory_dir = self.memory_dir.display(),
        )
    }
}

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_dream(tmp: &TempDir) -> AutoDream {
        let mem = tmp.path().join("memory");
        let conv = tmp.path().join("conversations");
        AutoDream::new(mem, conv)
    }

    async fn completion_task(dream: &AutoDream) -> ConsolidationTask {
        ConsolidationTask {
            prompt: String::new(),
            memory_dir: dream.memory_dir.clone(),
            state_file: dream.state_file.clone(),
            lock_file: dream.lock_file.clone(),
            lock: dream.acquire_lock().await.unwrap(),
        }
    }

    #[tokio::test]
    async fn only_successful_execution_stamps_state() {
        use crate::agent_tool::AgentCompletion;
        for completion in [
            AgentCompletion::Succeeded,
            AgentCompletion::Partial,
            AgentCompletion::Cancelled,
            AgentCompletion::Failed,
        ] {
            let tmp = TempDir::new().unwrap();
            let dream = make_dream(&tmp);
            let task = completion_task(&dream).await;
            let (tx, rx) = tokio::sync::oneshot::channel();
            tx.send(completion).unwrap();
            AutoDream::finish_after_execution(
                task,
                rx,
                Default::default(),
                Default::default(),
                "s".into(),
            )
            .await
            .unwrap();
            assert_eq!(
                dream.load_state().await.last_consolidated_at.is_some(),
                completion == AgentCompletion::Succeeded
            );
            assert!(dream.acquire_lock().await.is_ok());
        }
    }

    #[tokio::test]
    async fn revoked_goal_cancels_but_waits_for_execution_acknowledgement() {
        use claurst_core::goal::{GoalStore, GoalTurnContext};
        for revocation in ["pause", "clear", "replace"] {
            let tmp = TempDir::new().unwrap();
            let dream = make_dream(&tmp);
            let task = completion_task(&dream).await;
            let store = GoalStore::open(&tmp.path().join("goals.sqlite")).unwrap();
            store.set_goal("s", "initial", None).unwrap();
            let goal = GoalTurnContext::Captured(store.capture_active_goal("s").unwrap())
                .for_background_work();
            let (tx, rx) = tokio::sync::oneshot::channel();
            let cancel = tokio_util::sync::CancellationToken::new();
            let waiter = tokio::spawn(AutoDream::finish_after_execution(
                task,
                rx,
                cancel.clone(),
                goal,
                "s".into(),
            ));
            match revocation {
                "pause" => {
                    store.pause_active_goal("s").unwrap();
                }
                "clear" => {
                    store.clear_goal("s").unwrap();
                }
                "replace" => {
                    store.clear_goal("s").unwrap();
                    store.set_goal("s", "replacement", None).unwrap();
                }
                _ => unreachable!(),
            }
            tokio::time::timeout(Duration::from_secs(2), cancel.cancelled())
                .await
                .unwrap();
            assert!(!waiter.is_finished());
            assert!(dream.acquire_lock().await.is_err());
            tx.send(crate::agent_tool::AgentCompletion::Succeeded)
                .unwrap();
            waiter.await.unwrap().unwrap();
            assert!(!dream.state_file.exists());
            assert!(dream.acquire_lock().await.is_ok());
        }
    }

    #[tokio::test]
    async fn closed_receipt_never_stamps_success() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        let task = completion_task(&dream).await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        drop(tx);
        assert!(AutoDream::finish_after_execution(
            task,
            rx,
            Default::default(),
            Default::default(),
            "s".into()
        )
        .await
        .is_err());
        assert!(!dream.state_file.exists());
    }

    #[tokio::test]
    async fn persistence_failure_keeps_live_lock_for_retry() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        let task = completion_task(&dream).await;
        fs::create_dir(&task.state_file).await.unwrap();
        assert!(AutoDream::finish_consolidation(&task).await.is_err());
        assert!(dream.acquire_lock().await.is_err());
        fs::remove_dir(&task.state_file).await.unwrap();
        AutoDream::finish_consolidation(&task).await.unwrap();
        assert!(dream.load_state().await.last_consolidated_at.is_some());
        drop(task);
        assert!(dream.acquire_lock().await.is_ok());
    }

    #[tokio::test]
    async fn persistence_retry_keeps_exclusion_until_state_can_be_written() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        let task = completion_task(&dream).await;
        fs::create_dir(&task.state_file).await.unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tx.send(crate::agent_tool::AgentCompletion::Succeeded)
            .unwrap();
        let mut waiter = tokio::spawn(AutoDream::finish_after_execution(
            task,
            rx,
            Default::default(),
            Default::default(),
            "s".into(),
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut waiter)
                .await
                .is_err(),
            "persistence failure must retain a retry owner"
        );
        assert!(dream.acquire_lock().await.is_err());
        assert!(dream.state_file.is_dir());
        fs::remove_dir(&dream.state_file).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(dream.load_state().await.last_consolidated_at.is_some());
        assert!(dream.acquire_lock().await.is_ok());
    }

    // --- time_gate_passes ---

    #[test]
    fn test_time_gate_never_consolidated() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        let state = ConsolidationState::default();
        assert!(
            dream.time_gate_passes(&state),
            "no prior consolidation → gate passes"
        );
    }

    #[test]
    fn test_time_gate_recent_consolidation() {
        let tmp = TempDir::new().unwrap();
        let dream = AutoDream::with_config(
            AutoDreamConfig {
                min_hours: 24.0,
                min_sessions: 5,
            },
            tmp.path().join("memory"),
            tmp.path().join("conversations"),
        );
        let state = ConsolidationState {
            last_consolidated_at: Some(now_secs()), // just now
            lock_etag: None,
        };
        assert!(
            !dream.time_gate_passes(&state),
            "just consolidated → gate blocked"
        );
    }

    #[test]
    fn test_time_gate_old_consolidation() {
        let tmp = TempDir::new().unwrap();
        let dream = AutoDream::with_config(
            AutoDreamConfig {
                min_hours: 24.0,
                min_sessions: 5,
            },
            tmp.path().join("memory"),
            tmp.path().join("conversations"),
        );
        // 25 hours ago
        let old = now_secs().saturating_sub(25 * 3600);
        let state = ConsolidationState {
            last_consolidated_at: Some(old),
            lock_etag: None,
        };
        assert!(
            dream.time_gate_passes(&state),
            "consolidated 25h ago → gate passes"
        );
    }

    // --- lock_gate_passes (sync-friendly via tokio::test) ---

    #[tokio::test]
    async fn test_lock_gate_no_lock_file() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        assert!(dream.lock_gate_passes().await.unwrap());
    }

    #[tokio::test]
    async fn test_lock_gate_fresh_lock_blocks() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        let _lease = dream.acquire_lock().await.unwrap();
        // A live owner, not the age of the pathname, blocks acquisition.
        assert!(!dream.lock_gate_passes().await.unwrap());
    }

    // --- consolidation_prompt sanity ---

    #[test]
    fn test_consolidation_prompt_contains_paths() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        let prompt = dream.consolidation_prompt();
        assert!(prompt.contains("MEMORY.md"));
        assert!(prompt.contains("Memory Consolidation"));
        assert!(prompt.contains("Phase 1"));
        assert!(prompt.contains("Phase 4"));
    }

    #[test]
    fn test_consolidation_prompt_excludes_transcript_access() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        let prompt = dream.consolidation_prompt();
        assert!(prompt.contains("Session transcripts are intentionally unavailable"));
        assert!(!prompt.contains(dream.conversations_dir.to_string_lossy().as_ref()));
        assert!(!prompt.contains("*.jsonl"));
        assert!(!prompt.contains("grep -rn"));
    }

    // --- update_state / load_state round-trip ---

    #[tokio::test]
    async fn test_state_round_trip() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        std::fs::create_dir_all(&dream.memory_dir).unwrap();

        let mut state = ConsolidationState::default();
        dream.update_state(&mut state).await.unwrap();

        assert!(state.last_consolidated_at.is_some());
        let loaded = dream.load_state().await;
        assert_eq!(loaded.last_consolidated_at, state.last_consolidated_at);
    }

    // --- acquire_lock / release_lock ---

    #[tokio::test]
    async fn lock_acquisition_rejects_a_second_owner() {
        let tmp = TempDir::new().unwrap();
        let first = make_dream(&tmp);
        let second = make_dream(&tmp);
        let _owner = first.acquire_lock().await.unwrap();

        assert!(
            second.acquire_lock().await.is_err(),
            "another owner must not acquire a live consolidation lock"
        );
    }

    #[tokio::test]
    async fn old_timestamp_does_not_override_a_live_execution_lease() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        let owner = dream.acquire_lock().await.unwrap();
        owner.set_modified(UNIX_EPOCH).unwrap();
        let execution_lease = Arc::clone(&owner);
        drop(owner);
        assert!(!dream.lock_gate_passes().await.unwrap());
        drop(execution_lease);
        assert!(dream.lock_gate_passes().await.unwrap());
    }

    #[tokio::test]
    async fn lock_excludes_another_process_until_owner_exit() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);
        let mut owner = Some(dream.acquire_lock().await.unwrap());
        for expected in ["blocked", "acquired"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "auto_dream::tests::lock_contender_process",
                    "--ignored",
                ])
                .env("COVEN_TEST_LOCK_ROOT", tmp.path())
                .env("COVEN_TEST_LOCK_EXPECTED", expected)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            drop(owner.take());
        }
        assert!(dream.lock_file.exists());
    }

    #[tokio::test]
    #[ignore = "isolated lock contender invoked by parent test"]
    async fn lock_contender_process() {
        let root = PathBuf::from(std::env::var_os("COVEN_TEST_LOCK_ROOT").unwrap());
        let dream = AutoDream::new(root.join("memory"), root.join("conversations"));
        let result = dream.acquire_lock().await;
        match std::env::var("COVEN_TEST_LOCK_EXPECTED").unwrap().as_str() {
            "acquired" => assert!(result.is_ok()),
            "blocked" => {
                let error = result.unwrap_err();
                assert_eq!(
                    error.downcast_ref::<std::io::Error>().unwrap().kind(),
                    std::io::ErrorKind::WouldBlock
                );
            }
            other => panic!("unexpected fixture expectation: {other}"),
        }
    }

    #[tokio::test]
    async fn test_acquire_release_lock() {
        let tmp = TempDir::new().unwrap();
        let dream = make_dream(&tmp);

        let lease = dream.acquire_lock().await.unwrap();
        assert!(dream.lock_file.exists());
        drop(lease);
        assert!(dream.lock_file.exists());
        let _next_owner = dream.acquire_lock().await.unwrap();
    }
}
