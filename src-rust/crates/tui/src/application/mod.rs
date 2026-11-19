//! Typed presentation seam shared by the Classic and Next renderers.
//!
//! The interactive loop in the CLI owns every runtime effect (query workers,
//! permission decisions, persistence). This module carries what crosses from
//! that loop into presentation: runtime observations scoped to the session
//! generation that produced them, the external effects a reducer step asks the
//! loop to perform, and the renderer selection. Renderers only read [`App`]
//! state and the [`view`] projection; neither renderer executes tools,
//! decides permissions or persists anything.
//!
//! [`App`]: crate::App

pub mod memory;
pub mod research;
pub mod view;
pub mod work;

use claurst_core::coven_daemon::{DaemonInstance, DaemonSighting};
use claurst_query::QueryEvent;

/// Environment variable that selects the interactive renderer.
pub const UI_SELECTOR_ENV: &str = "COVEN_CODE_UI";

/// Identifies the session scope a runtime observation belongs to.
///
/// The interactive loop advances the generation whenever the visible
/// conversation is replaced (clear, rewind, resume). Observations produced by
/// a worker spawned under an older generation are discarded by
/// [`SessionScope::admits`] instead of mutating the new conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SessionGeneration(u64);

/// Tracks the current session generation for the interactive loop.
#[derive(Debug, Default)]
pub struct SessionScope {
    current: SessionGeneration,
}

impl SessionScope {
    pub fn current(&self) -> SessionGeneration {
        self.current
    }

    /// Start a new generation. Observations tagged with any earlier
    /// generation are no longer admitted.
    pub fn advance(&mut self) -> SessionGeneration {
        self.current = SessionGeneration(self.current.0.wrapping_add(1));
        self.current
    }

    pub fn admits(&self, generation: SessionGeneration) -> bool {
        generation == self.current
    }

    /// Apply a scoped observation to presentation state when it belongs to
    /// the current generation; stale observations change nothing.
    pub fn apply(&self, app: &mut crate::App, scoped: ScopedQueryEvent) -> Vec<PresentationEffect> {
        if !self.admits(scoped.generation) {
            tracing::debug!("discarding query event from a previous session generation");
            return Vec::new();
        }
        app.apply_query_event(scoped.event)
    }
}

/// A runtime observation tagged with the generation of the worker that
/// produced it.
#[derive(Debug)]
pub struct ScopedQueryEvent {
    pub generation: SessionGeneration,
    pub event: QueryEvent,
}

/// Capacity of the delivery queue between query workers and the interactive
/// loop.
///
/// Each worker's forwarder waits for space instead of dropping, so semantic
/// observations (tool results, errors, turn completion, durable messages)
/// are never discarded under pressure. A full queue stops the forwarder,
/// which fills the worker's per-turn buffer
/// ([`QUERY_PRODUCER_CAPACITY`]), which pauses the query loop itself.
pub const QUERY_EVENT_QUEUE_CAPACITY: usize = 1024;

/// Capacity of one turn's buffer between the query loop and its forwarder.
///
/// The query loop waits for space here rather than dropping or growing, so a
/// stalled interactive loop pauses the turn: about this many events plus
/// [`QUERY_EVENT_QUEUE_CAPACITY`] are held for it (plus the one event the
/// forwarder holds and one per concurrent tool call waiting to send). 256 matches the API
/// client's own stream channel, which feeds most of these events one for one,
/// so a full upstream stream fits without the loop waiting in normal use.
pub const QUERY_PRODUCER_CAPACITY: usize = 256;

/// An external effect requested by a presentation reducer step.
///
/// Reducer steps only change [`App`](crate::App) state; anything that
/// reaches outside the process state is returned here and performed by the
/// interactive loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationEffect {
    /// Write the terminal bell for a completed long-running turn.
    RingBell,
}

/// Perform effects returned by a reducer step.
pub fn perform_effects(effects: &[PresentationEffect]) {
    for effect in effects {
        match effect {
            PresentationEffect::RingBell => crate::app::ring_terminal_bell(),
        }
    }
}

/// Where a user's request to cancel the running turn stands.
///
/// Presentation records the request; the interactive loop performs it
/// through the turn's cancellation token and settles it once the worker has
/// actually stopped. Until then the request is pending, not done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnCancellation {
    #[default]
    Idle,
    /// Asked for by a key; the loop has not acted yet.
    Requested,
    /// The loop cancelled the turn's token; the worker has not finished.
    AwaitingRuntime,
}

/// Who owns this session's execution, as far as a producer has shown.
///
/// Registration in the Coven daemon ledger is best effort and records the
/// session; it is not daemon control or identity enforcement. Nothing here
/// claims daemon-managed execution, which no producer offers yet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SessionOwnership {
    /// Run and enforced by this process; no ledger registration requested.
    #[default]
    EngineLocal,
    /// Registration of `session_id` was sent; no answer yet.
    LedgerPending { session_id: String },
    /// Registration of `session_id` was sent again at a session start,
    /// because the earlier one was unknown; no answer yet. Until a named
    /// instance accepts it, the daemon indicator still reads unknown.
    LedgerRenewing { session_id: String },
    /// The daemon `instance` accepted the registration of `session_id`.
    /// A sighting taken before `since` says nothing about it.
    LedgerRegistered {
        session_id: String,
        instance: DaemonInstance,
        since: std::time::Instant,
    },
    /// The registration of `session_id` failed or no daemon was reachable.
    LedgerFailed { session_id: String },
    /// A daemon accepted the registration of `session_id`, but it is not
    /// known to be the one answering now: another instance has answered
    /// since, or the one that accepted it was never identified. It stays
    /// unknown until a session start registers again; nothing is resent
    /// before that.
    LedgerUnknown { session_id: String },
}

/// The daemon's answer to a ledger registration (see
/// `claurst_core::coven_ledger::notify_session_start`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationAnswer {
    /// Accepted, and this instance answered before and after it.
    Held(DaemonInstance),
    /// Accepted, but no single instance can be named for it.
    Unnamed,
    /// Refused, or no daemon answered.
    Failed,
}

impl From<Result<Option<DaemonInstance>, String>> for RegistrationAnswer {
    fn from(result: Result<Option<DaemonInstance>, String>) -> Self {
        match result {
            Ok(Some(instance)) => RegistrationAnswer::Held(instance),
            Ok(None) => RegistrationAnswer::Unnamed,
            Err(_) => RegistrationAnswer::Failed,
        }
    }
}

/// How the daemon indicator reads: the welcome line and Classic's footer.
/// It is about this process's registration, whichever session is bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonPresence {
    /// No daemon answers.
    Offline,
    /// A daemon answers, and nothing this session holds contradicts it.
    Online,
    /// A daemon answers, but not one known to hold the registration this
    /// process sent.
    Unknown,
}

impl SessionOwnership {
    /// A short label and a one-line explanation for the session `current`.
    /// A registration for a different session says nothing about this one.
    pub fn describe(&self, current: Option<&str>) -> (&'static str, &'static str) {
        let applies = |id: &str| current == Some(id);
        match self {
            SessionOwnership::LedgerPending { session_id } if applies(session_id) => (
                "engine-local, registering",
                "runs in this process; ledger registration has not answered yet",
            ),
            SessionOwnership::LedgerRenewing { session_id } if applies(session_id) => (
                "engine-local, registering",
                "runs in this process; the ledger registration was sent again and has not answered yet",
            ),
            SessionOwnership::LedgerRegistered { session_id, .. } if applies(session_id) => (
                "engine-local, ledger-registered",
                "runs and is enforced in this process; the daemon records it but does not control it",
            ),
            SessionOwnership::LedgerFailed { session_id } if applies(session_id) => (
                "engine-local, ledger unavailable",
                "runs in this process; ledger registration failed, so the daemon does not know this session",
            ),
            SessionOwnership::LedgerUnknown { session_id } if applies(session_id) => (
                "engine-local, ledger unknown",
                "runs in this process; the daemon restarted or is unidentified, so its record is unknown until the next session start",
            ),
            SessionOwnership::EngineLocal => (
                "engine-local",
                "runs and is enforced in this process; not registered with a daemon",
            ),
            _ => (
                "engine-local",
                "runs in this process; the ledger registration was for a different session",
            ),
        }
    }

    /// Record the daemon's answer to a registration that was waiting for
    /// one. A first registration that fails is failed, as before. One sent
    /// again after the earlier one became unknown goes back to unknown when
    /// it fails, so the next session start tries again.
    pub fn settle_registration(&mut self, answer: RegistrationAnswer, now: std::time::Instant) {
        let (session_id, renewing) = match std::mem::take(self) {
            SessionOwnership::LedgerPending { session_id } => (session_id, false),
            SessionOwnership::LedgerRenewing { session_id } => (session_id, true),
            settled => {
                *self = settled;
                return;
            }
        };
        *self = match answer {
            RegistrationAnswer::Held(instance) => SessionOwnership::LedgerRegistered {
                session_id,
                instance,
                since: now,
            },
            RegistrationAnswer::Unnamed => SessionOwnership::LedgerUnknown { session_id },
            RegistrationAnswer::Failed if renewing => {
                SessionOwnership::LedgerUnknown { session_id }
            }
            RegistrationAnswer::Failed => SessionOwnership::LedgerFailed { session_id },
        };
    }

    /// Compare what a probe that began at `seen_at` found with the instance
    /// that holds the registration. A different instance makes the registration
    /// unknown, and it stays so. A daemon that is gone or did not identify
    /// itself changes nothing: only a named, different instance does.
    pub fn observe_daemon(&mut self, sighting: &DaemonSighting, seen_at: std::time::Instant) {
        let SessionOwnership::LedgerRegistered {
            instance, since, ..
        } = &*self
        else {
            return;
        };
        let replaced = matches!(sighting, DaemonSighting::Instance(seen) if seen != instance);
        if !replaced || seen_at < *since {
            return;
        }
        if let SessionOwnership::LedgerRegistered { session_id, .. } = std::mem::take(self) {
            *self = SessionOwnership::LedgerUnknown { session_id };
        }
    }

    /// Whether a session start should register again: only after the
    /// registration became unknown. Nothing registers between session
    /// starts.
    pub fn registration_is_unknown(&self) -> bool {
        matches!(self, SessionOwnership::LedgerUnknown { .. })
    }

    /// The daemon indicator, given whether a daemon answers at all. It
    /// reads unknown from the moment the registration does until a named
    /// instance has accepted a new one: sending it again is not enough.
    pub fn daemon_presence(&self, online: bool) -> DaemonPresence {
        let unknown = matches!(
            self,
            SessionOwnership::LedgerUnknown { .. } | SessionOwnership::LedgerRenewing { .. }
        );
        if !online {
            DaemonPresence::Offline
        } else if unknown {
            DaemonPresence::Unknown
        } else {
            DaemonPresence::Online
        }
    }
}

/// The reader's place in one session, kept while another session is shown.
#[derive(Debug, Clone, Default)]
pub struct ScopeView {
    /// The unsent draft. Restored to the composer, never sent on restore.
    pub draft: String,
    /// Bodies of the draft's `[Pasted ...]` placeholders, by paste id.
    pub pastes: std::collections::HashMap<u32, String>,
    pub images: Vec<crate::image_paste::PastedImage>,
    pub scroll_offset: usize,
    pub auto_scroll: bool,
}

/// Interactive renderer selected by [`UI_SELECTOR_ENV`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiRenderer {
    /// The existing renderer. Default.
    #[default]
    Classic,
    /// Opt-in preview renderer over the same application state.
    Next,
}

/// Result of resolving the renderer selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererSelection {
    pub renderer: UiRenderer,
    /// Present when the selector held an unsupported value; the caller
    /// reports it on stderr before the terminal is initialized.
    pub warning: Option<String>,
}

/// Resolve the renderer from the raw selector value.
///
/// Unset or empty selects Classic. Values are matched case-insensitively
/// after trimming. Anything else selects Classic with a warning naming the
/// accepted values. Only the interactive route calls this; headless,
/// stream-json, ACP and named-command routes never read the selector.
pub fn select_renderer(raw: Option<&str>) -> RendererSelection {
    let value = raw.map(str::trim).unwrap_or_default();
    let renderer = if value.is_empty() || value.eq_ignore_ascii_case("classic") {
        UiRenderer::Classic
    } else if value.eq_ignore_ascii_case("next") {
        UiRenderer::Next
    } else {
        return RendererSelection {
            renderer: UiRenderer::Classic,
            warning: Some(format!(
                "{UI_SELECTOR_ENV}={value:?} is not supported (expected \"classic\" or \"next\"); starting Classic"
            )),
        };
    };
    RendererSelection {
        renderer,
        warning: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_rejects_observations_from_earlier_generations() {
        let mut scope = SessionScope::default();
        let first = scope.current();
        assert!(scope.admits(first));
        let second = scope.advance();
        assert_ne!(first, second);
        assert!(!scope.admits(first), "stale generation must be rejected");
        assert!(scope.admits(second));
        scope.advance();
        assert!(!scope.admits(second));
    }

    fn text_delta(text: &str) -> QueryEvent {
        QueryEvent::Stream(claurst_api::AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: claurst_api::streaming::ContentDelta::TextDelta {
                text: text.to_string(),
            },
        })
    }

    #[test]
    fn stale_worker_output_does_not_reach_a_replaced_conversation() {
        let mut app = crate::App::new(
            claurst_core::config::Config::default(),
            claurst_core::cost::CostTracker::new(),
        );
        let mut scope = SessionScope::default();
        let old = scope.current();
        scope.apply(
            &mut app,
            ScopedQueryEvent {
                generation: old,
                event: text_delta("before clear "),
            },
        );
        assert_eq!(app.streaming_text, "before clear ");

        // `/clear` replaces the conversation and starts a new generation.
        app.replace_messages(Vec::new());
        app.streaming_text.clear();
        app.is_streaming = false;
        let new = scope.advance();

        // Output still queued from the old worker arrives late.
        let effects = scope.apply(
            &mut app,
            ScopedQueryEvent {
                generation: old,
                event: text_delta("late"),
            },
        );
        assert!(effects.is_empty());
        assert_eq!(
            app.streaming_text, "",
            "stale delta leaked into new conversation"
        );
        assert!(!app.is_streaming, "stale delta restarted streaming state");

        scope.apply(
            &mut app,
            ScopedQueryEvent {
                generation: new,
                event: text_delta("fresh"),
            },
        );
        assert_eq!(app.streaming_text, "fresh");
    }

    fn instance(pid: u32, started_at: &str) -> DaemonInstance {
        DaemonInstance {
            pid: Some(pid),
            started_at: Some(started_at.to_string()),
            api_version: "coven.daemon.v1".to_string(),
            coven_version: "0.5.0".to_string(),
            socket_file: Some((1, u64::from(pid))),
        }
    }

    fn registered(now: std::time::Instant) -> SessionOwnership {
        let mut ownership = SessionOwnership::LedgerPending {
            session_id: "session-a".to_string(),
        };
        ownership.settle_registration(RegistrationAnswer::Held(instance(100, "05:00:00Z")), now);
        ownership
    }

    fn unknown() -> SessionOwnership {
        SessionOwnership::LedgerUnknown {
            session_id: "session-a".to_string(),
        }
    }

    /// #412, policy B: the registration follows the daemon instance that
    /// took it.
    #[test]
    fn the_same_instance_keeps_the_registration() {
        let start = std::time::Instant::now();
        let later = start + std::time::Duration::from_secs(10);
        let mut ownership = registered(start);
        let before = ownership.clone();
        ownership.observe_daemon(&DaemonSighting::Instance(instance(100, "05:00:00Z")), later);
        assert_eq!(ownership, before);
        assert_eq!(ownership.daemon_presence(true), DaemonPresence::Online);
        assert!(!ownership.registration_is_unknown());
    }

    #[test]
    fn a_changed_instance_makes_the_registration_unknown_and_it_stays_unknown() {
        let start = std::time::Instant::now();
        let later = start + std::time::Duration::from_secs(10);
        for changed in [
            instance(101, "05:00:00Z"),
            instance(100, "05:00:09Z"),
            DaemonInstance {
                socket_file: Some((1, 999)),
                ..instance(100, "05:00:00Z")
            },
            DaemonInstance {
                coven_version: "0.5.1".to_string(),
                ..instance(100, "05:00:00Z")
            },
        ] {
            let mut ownership = registered(start);
            ownership.observe_daemon(&DaemonSighting::Instance(changed.clone()), later);
            assert_eq!(ownership, unknown(), "{changed:?}");
            assert_eq!(ownership.daemon_presence(true), DaemonPresence::Unknown);
            assert_eq!(ownership.daemon_presence(false), DaemonPresence::Offline);
            assert_eq!(
                ownership.describe(Some("session-a")).0,
                "engine-local, ledger unknown"
            );

            // Nothing a later probe finds settles it, not even the first
            // instance again: only a session start registers again.
            for sighting in [
                DaemonSighting::Instance(instance(100, "05:00:00Z")),
                DaemonSighting::Instance(changed.clone()),
                DaemonSighting::Unidentified,
                DaemonSighting::Gone,
            ] {
                ownership.observe_daemon(&sighting, later);
                assert_eq!(ownership, unknown(), "{sighting:?}");
            }
            assert!(ownership.registration_is_unknown());
        }
    }

    #[test]
    fn a_daemon_that_is_gone_and_back_as_the_same_instance_keeps_the_registration() {
        let start = std::time::Instant::now();
        let later = start + std::time::Duration::from_secs(10);
        let mut ownership = registered(start);
        let before = ownership.clone();

        // Gone: the registration is neither confirmed nor contradicted, and
        // the indicator reads offline while nothing answers.
        ownership.observe_daemon(&DaemonSighting::Gone, later);
        assert_eq!(ownership, before);
        assert_eq!(ownership.daemon_presence(false), DaemonPresence::Offline);

        // Busy, or answering without naming itself: no evidence either way.
        ownership.observe_daemon(&DaemonSighting::Unidentified, later);
        assert_eq!(ownership, before);

        // Back, and the same process: still registered.
        ownership.observe_daemon(&DaemonSighting::Instance(instance(100, "05:00:00Z")), later);
        assert_eq!(ownership, before);
        assert_eq!(ownership.daemon_presence(true), DaemonPresence::Online);

        // Back as another process: unknown.
        ownership.observe_daemon(&DaemonSighting::Gone, later);
        ownership.observe_daemon(&DaemonSighting::Instance(instance(200, "05:01:00Z")), later);
        assert_eq!(ownership, unknown());
    }

    #[test]
    fn a_session_that_never_registered_is_not_changed_by_any_sighting() {
        let now = std::time::Instant::now();
        for start in [
            SessionOwnership::EngineLocal,
            SessionOwnership::LedgerPending {
                session_id: "session-a".to_string(),
            },
            SessionOwnership::LedgerFailed {
                session_id: "session-a".to_string(),
            },
        ] {
            let mut ownership = start.clone();
            for sighting in [
                DaemonSighting::Instance(instance(100, "05:00:00Z")),
                DaemonSighting::Instance(instance(200, "05:01:00Z")),
                DaemonSighting::Unidentified,
                DaemonSighting::Gone,
            ] {
                ownership.observe_daemon(&sighting, now);
                assert_eq!(ownership, start, "{sighting:?}");
            }
            assert!(!ownership.registration_is_unknown(), "{start:?}");
            assert_eq!(ownership.daemon_presence(true), DaemonPresence::Online);
            assert_eq!(ownership.daemon_presence(false), DaemonPresence::Offline);
        }
    }

    /// A probe that began before the registration was recorded may show
    /// the daemon from before a restart; it must not contradict a
    /// registration the restarted daemon took.
    #[test]
    fn a_sighting_older_than_the_registration_is_ignored() {
        let start = std::time::Instant::now();
        let registered_at = start + std::time::Duration::from_secs(5);
        let mut ownership = registered(registered_at);
        let before = ownership.clone();
        let other = DaemonSighting::Instance(instance(50, "04:00:00Z"));
        ownership.observe_daemon(&other, start);
        assert_eq!(ownership, before);
        ownership.observe_daemon(&other, registered_at);
        assert_eq!(ownership, unknown());
    }

    #[test]
    fn a_registration_answer_settles_only_a_pending_registration() {
        let now = std::time::Instant::now();
        let pending = || SessionOwnership::LedgerPending {
            session_id: "session-a".to_string(),
        };

        let mut ownership = pending();
        ownership.settle_registration(RegistrationAnswer::Failed, now);
        assert_eq!(
            ownership,
            SessionOwnership::LedgerFailed {
                session_id: "session-a".to_string()
            }
        );

        let mut ownership = pending();
        ownership.settle_registration(RegistrationAnswer::Held(instance(100, "05:00:00Z")), now);
        assert_eq!(ownership, registered(now));

        // Accepted, but by no instance that can be named: unknown at once.
        let mut ownership = pending();
        ownership.settle_registration(RegistrationAnswer::Unnamed, now);
        assert_eq!(ownership, unknown());

        // A late or repeated answer changes no state that is not waiting
        // for one.
        for settled in [
            SessionOwnership::EngineLocal,
            registered(now),
            unknown(),
            SessionOwnership::LedgerFailed {
                session_id: "session-a".to_string(),
            },
        ] {
            let mut ownership = settled.clone();
            ownership
                .settle_registration(RegistrationAnswer::Held(instance(300, "06:00:00Z")), now);
            assert_eq!(ownership, settled);
            ownership.settle_registration(RegistrationAnswer::Failed, now);
            assert_eq!(ownership, settled);
        }
    }

    /// A registration sent again after the earlier one became unknown keeps
    /// the indicator at unknown until a named instance accepts it, and goes
    /// back to unknown, not to failed, when it is refused or lost.
    #[test]
    fn a_registration_sent_again_reads_unknown_until_an_instance_accepts_it() {
        let now = std::time::Instant::now();
        let renewing = || SessionOwnership::LedgerRenewing {
            session_id: "session-a".to_string(),
        };
        assert_eq!(renewing().daemon_presence(true), DaemonPresence::Unknown);
        assert_eq!(renewing().daemon_presence(false), DaemonPresence::Offline);
        assert!(!renewing().registration_is_unknown(), "it is already sent");
        assert_eq!(
            renewing().describe(Some("session-a")).0,
            "engine-local, registering"
        );

        // No probe settles it: only the answer does.
        let mut ownership = renewing();
        ownership.observe_daemon(&DaemonSighting::Instance(instance(200, "05:01:00Z")), now);
        assert_eq!(ownership, renewing());

        let mut ownership = renewing();
        ownership.settle_registration(RegistrationAnswer::Held(instance(100, "05:00:00Z")), now);
        assert_eq!(ownership, registered(now));
        assert_eq!(ownership.daemon_presence(true), DaemonPresence::Online);

        for lost in [RegistrationAnswer::Failed, RegistrationAnswer::Unnamed] {
            let mut ownership = renewing();
            ownership.settle_registration(lost.clone(), now);
            assert_eq!(ownership, unknown(), "{lost:?}");
            assert!(ownership.registration_is_unknown());
            assert_eq!(ownership.daemon_presence(true), DaemonPresence::Unknown);
        }
    }

    /// The label is about the bound session; the indicator is about the
    /// registration this process sent, whichever session is bound.
    #[test]
    fn an_unknown_registration_for_another_session_still_reads_unknown_in_the_indicator() {
        let ownership = unknown();
        assert_eq!(ownership.describe(Some("session-b")).0, "engine-local");
        assert_eq!(ownership.daemon_presence(true), DaemonPresence::Unknown);
    }

    #[test]
    fn registration_answers_come_from_the_ledger_call() {
        let held = instance(100, "05:00:00Z");
        assert_eq!(
            RegistrationAnswer::from(Ok(Some(held.clone()))),
            RegistrationAnswer::Held(held)
        );
        assert_eq!(
            RegistrationAnswer::from(Ok(None)),
            RegistrationAnswer::Unnamed
        );
        assert_eq!(
            RegistrationAnswer::from(Err("Coven daemon offline".to_string())),
            RegistrationAnswer::Failed
        );
    }

    #[test]
    fn selector_defaults_to_classic_without_warning() {
        for raw in [None, Some(""), Some("  ")] {
            assert_eq!(
                select_renderer(raw),
                RendererSelection {
                    renderer: UiRenderer::Classic,
                    warning: None
                },
                "raw={raw:?}"
            );
        }
    }

    #[test]
    fn selector_accepts_classic_and_next_case_insensitively() {
        assert_eq!(
            select_renderer(Some("classic")).renderer,
            UiRenderer::Classic
        );
        assert_eq!(
            select_renderer(Some("CLASSIC")).renderer,
            UiRenderer::Classic
        );
        let next = select_renderer(Some(" Next "));
        assert_eq!(next.renderer, UiRenderer::Next);
        assert_eq!(next.warning, None);
    }

    #[test]
    fn selector_rejects_unknown_values_with_classic_and_warning() {
        let selection = select_renderer(Some("fancy"));
        assert_eq!(selection.renderer, UiRenderer::Classic);
        let warning = selection.warning.expect("unknown value must warn");
        assert!(warning.contains(UI_SELECTOR_ENV), "{warning}");
        assert!(warning.contains("fancy"), "{warning}");
        assert!(
            warning.contains("classic") && warning.contains("next"),
            "{warning}"
        );
    }
}
