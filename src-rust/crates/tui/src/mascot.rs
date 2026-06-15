//! Companion pose state for the familiar card.
//!
//! The active familiar's glyph is composed by [`crate::familiar_card`] from the
//! procedural sigil for its archetype (resolved in [`crate::familiar_theme`]).
//! No named persona or bespoke pixel-art is baked into the binary, so a fresh
//! install never inherits a built-in familiar.
//!
//! This module only carries the pose/expression the card animates in: surfaces
//! that never animate pass [`CompanionPose::Static`]; the welcome panel passes
//! a live [`CompanionPose::Idle`] / [`CompanionPose::Loading`] whose frame
//! counter drives the card's subtle palette pulse.

/// Pose / expression of the companion mascot.
///
/// `Static` is the resting frame, used by surfaces that never animate (F2
/// switcher rows, `/agents` detail view). `Idle` and `Loading` carry a
/// monotonically-increasing frame counter: `Idle` drives the resting pulse,
/// `Loading` drives a faster pulse while the assistant is mid-turn and stalled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompanionPose {
    Static,
    Idle { frame: u64 },
    Loading { frame: u64 },
}
