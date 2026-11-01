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

pub mod view;
pub mod work;

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
/// are never discarded under pressure. The query loop itself still emits
/// into an unbounded per-turn buffer (`run_query_loop` takes an
/// `UnboundedSender`), so a stalled interactive loop backs up into that
/// buffer, not into this queue; bounding the producer needs a query-crate
/// API change.
pub const QUERY_EVENT_QUEUE_CAPACITY: usize = 1024;

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
