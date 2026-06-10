// effort.rs — EffortLevel enum and associated helpers.
//
// Maps to src/utils/effort.ts in the TypeScript source.  The Rust port
// retains only the subset of logic that is useful in a non-browser / non-GrowthBook
// environment: the level → thinking-budget / temperature / glyph mappings.
//
// The thinking-budget and temperature values must match the TypeScript source
// exactly because they are passed to the Anthropic API.

// ---------------------------------------------------------------------------
// Adaptive-thinking model detection
// ---------------------------------------------------------------------------

/// Returns `true` for Anthropic models that use *adaptive* extended thinking
/// and have removed the manual `budget_tokens` and sampling parameters
/// (`temperature` / `top_p` / `top_k`).
///
/// On these models (Claude Opus 4.7, Opus 4.8, and Fable 5) sending
/// `thinking: {type: "enabled", budget_tokens: N}` or a `temperature` returns
/// a 400 error. Callers must instead send `thinking: {type: "adaptive"}` and
/// control reasoning depth with the `effort` parameter. See the Anthropic
/// model-migration guide for the full breaking-change list.
pub fn model_uses_adaptive_thinking(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.contains("fable")
        || m.contains("claude-opus-4-7")
        || m.contains("opus-4-7")
        || m.contains("claude-opus-4-8")
        || m.contains("opus-4-8")
}

// ---------------------------------------------------------------------------
// EffortLevel enum
// ---------------------------------------------------------------------------

/// The four named effort levels supported by Coven Code.
///
/// Matches the `EffortLevel` type from `src/entrypoints/sdk/runtimeTypes.ts`
/// / `src/utils/effort.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffortLevel {
    /// Quick, straightforward implementation with minimal overhead.
    Low,
    /// Balanced approach with standard implementation and testing.
    Medium,
    /// Comprehensive implementation with extensive testing and documentation.
    High,
    /// Maximum capability with deepest reasoning (Opus-tier and Fable models).
    Max,
}

impl EffortLevel {
    /// Parse an effort level from its string representation.
    ///
    /// Accepts lowercase strings: `"low"`, `"medium"`, `"high"`, `"max"`.
    /// Returns `None` for any other value.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    /// The lowercase string name of this effort level.
    ///
    /// Round-trips with `from_str`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    /// Return the extended-thinking budget in tokens for this effort level,
    /// or `None` if thinking should be disabled.
    ///
    /// Values mirror the TypeScript `thinkingBudgetForEffort` mapping:
    ///   Low    → None  (no thinking)
    ///   Medium → 5 000
    ///   High   → 10 000
    ///   Max    → 20 000
    pub fn thinking_budget_tokens(&self) -> Option<u32> {
        match self {
            Self::Low => None,
            Self::Medium => Some(5_000),
            Self::High => Some(10_000),
            Self::Max => Some(20_000),
        }
    }

    /// Return the temperature override for this effort level, or `None` to
    /// use the model's default.
    ///
    /// Values mirror the TypeScript source:
    ///   Low    → Some(0.0) — deterministic, cheap
    ///   Medium → None      — model default
    ///   High   → None      — model default
    ///   Max    → None      — model default
    pub fn temperature(&self) -> Option<f32> {
        match self {
            Self::Low => Some(0.0),
            Self::Medium | Self::High | Self::Max => None,
        }
    }

    /// A single Unicode glyph used to represent this effort level in the TUI.
    ///
    /// Glyphs mirror the TypeScript EffortCallout / status-bar rendering:
    ///   Low    → "○"  (empty circle)
    ///   Medium → "◐"  (half circle)
    ///   High   → "●"  (filled circle)
    ///   Max    → "◉"  (circled circle)
    pub fn glyph(&self) -> &'static str {
        match self {
            Self::Low => "○",
            Self::Medium => "◐",
            Self::High => "●",
            Self::Max => "◉",
        }
    }

    /// Human-readable description of this effort level.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Low => "Quick, straightforward implementation with minimal overhead",
            Self::Medium => "Balanced approach with standard implementation and testing",
            Self::High => "Comprehensive implementation with extensive testing and documentation",
            Self::Max => "Maximum capability with deepest reasoning (Opus 4.6 only)",
        }
    }
}

impl std::fmt::Display for EffortLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_roundtrips() {
        for level in [
            EffortLevel::Low,
            EffortLevel::Medium,
            EffortLevel::High,
            EffortLevel::Max,
        ] {
            let parsed = EffortLevel::parse(level.as_str());
            assert_eq!(
                parsed,
                Some(level),
                "from_str({:?}) should round-trip",
                level
            );
        }
    }

    #[test]
    fn from_str_case_insensitive() {
        assert_eq!(EffortLevel::parse("LOW"), Some(EffortLevel::Low));
        assert_eq!(EffortLevel::parse("Medium"), Some(EffortLevel::Medium));
        assert_eq!(EffortLevel::parse("HIGH"), Some(EffortLevel::High));
        assert_eq!(EffortLevel::parse("Max"), Some(EffortLevel::Max));
    }

    #[test]
    fn from_str_unknown_returns_none() {
        assert_eq!(EffortLevel::parse("ultra"), None);
        assert_eq!(EffortLevel::parse(""), None);
        assert_eq!(EffortLevel::parse("3"), None);
    }

    #[test]
    fn thinking_budget_matches_ts() {
        assert_eq!(EffortLevel::Low.thinking_budget_tokens(), None);
        assert_eq!(EffortLevel::Medium.thinking_budget_tokens(), Some(5_000));
        assert_eq!(EffortLevel::High.thinking_budget_tokens(), Some(10_000));
        assert_eq!(EffortLevel::Max.thinking_budget_tokens(), Some(20_000));
    }

    #[test]
    fn temperature_matches_ts() {
        // Low → 0.0 (deterministic)
        assert_eq!(EffortLevel::Low.temperature(), Some(0.0));
        // All others → None (model default)
        assert_eq!(EffortLevel::Medium.temperature(), None);
        assert_eq!(EffortLevel::High.temperature(), None);
        assert_eq!(EffortLevel::Max.temperature(), None);
    }

    #[test]
    fn glyphs_match_ts() {
        assert_eq!(EffortLevel::Low.glyph(), "○");
        assert_eq!(EffortLevel::Medium.glyph(), "◐");
        assert_eq!(EffortLevel::High.glyph(), "●");
        assert_eq!(EffortLevel::Max.glyph(), "◉");
    }

    #[test]
    fn adaptive_thinking_models_detected() {
        // Models that removed budget_tokens / sampling params.
        assert!(model_uses_adaptive_thinking("claude-opus-4-8"));
        assert!(model_uses_adaptive_thinking("claude-opus-4-7"));
        assert!(model_uses_adaptive_thinking("claude-fable-5"));
        assert!(model_uses_adaptive_thinking("anthropic/claude-opus-4-8"));
        // Models that still accept manual budget_tokens.
        assert!(!model_uses_adaptive_thinking("claude-opus-4-6"));
        assert!(!model_uses_adaptive_thinking("claude-sonnet-4-6"));
        assert!(!model_uses_adaptive_thinking("claude-haiku-4-5"));
    }

    #[test]
    fn display_matches_as_str() {
        for level in [
            EffortLevel::Low,
            EffortLevel::Medium,
            EffortLevel::High,
            EffortLevel::Max,
        ] {
            assert_eq!(format!("{}", level), level.as_str());
        }
    }
}
