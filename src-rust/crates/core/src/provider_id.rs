// provider_id.rs — Branded newtypes for provider and model identifiers.
//
// ProviderId and ModelId are separate newtype wrappers around String so that
// the type system prevents accidentally passing a model name where a provider
// name is expected (and vice-versa).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;

// ---------------------------------------------------------------------------
// ProviderId
// ---------------------------------------------------------------------------

/// A branded identifier for an LLM provider (e.g. "anthropic", "openai").
///
/// Well-known constants are provided as associated constants so callers do
/// not need to hard-code raw strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderId(String);

impl ProviderId {
    /// Construct a `ProviderId` from any string-like value.
    pub fn new(s: impl Into<String>) -> Self {
        ProviderId(s.into())
    }

    // -----------------------------------------------------------------------
    // Well-known provider constants
    // -----------------------------------------------------------------------

    pub const ANTHROPIC: &'static str = "anthropic";
    pub const CODEX: &'static str = "codex";
    /// Underlying vendor id for Codex's gpt-* models.
    pub const OPENAI: &'static str = "openai";
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Deref for ProviderId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<String> for ProviderId {
    fn from(s: String) -> Self {
        ProviderId(s)
    }
}

impl From<&str> for ProviderId {
    fn from(s: &str) -> Self {
        ProviderId(s.to_string())
    }
}

impl PartialEq<str> for ProviderId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for ProviderId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

// ---------------------------------------------------------------------------
// ModelId
// ---------------------------------------------------------------------------

/// A branded identifier for a model (e.g. "claude-opus-4-5", "gpt-4o").
///
/// Kept separate from `ProviderId` for type safety — you cannot accidentally
/// pass a model name where a provider name is expected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId(String);

impl ModelId {
    /// Construct a `ModelId` from any string-like value.
    pub fn new(s: impl Into<String>) -> Self {
        ModelId(s.into())
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Deref for ModelId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<String> for ModelId {
    fn from(s: String) -> Self {
        ModelId(s)
    }
}

impl From<&str> for ModelId {
    fn from(s: &str) -> Self {
        ModelId(s.to_string())
    }
}

impl PartialEq<str> for ModelId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for ModelId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}
