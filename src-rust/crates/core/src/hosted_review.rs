use serde::{Deserialize, Serialize};

/// Runtime isolation mode for a Coven Code session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeMode {
    #[default]
    Local,
    HostedReview,
}

impl RuntimeMode {
    pub fn is_hosted_review(self) -> bool {
        matches!(self, Self::HostedReview)
    }
}

/// Settings-backed hosted review configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HostedReviewConfig {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
}

impl HostedReviewConfig {
    pub fn is_default(&self) -> bool {
        !self.enabled
    }
}

/// Tenant/repository identity required before hosted mode may persist
/// durable memory or transcript artifacts into hosted namespaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedReviewScope {
    pub tenant_id: String,
    pub canonical_repo_identity: String,
}

impl HostedReviewScope {
    pub fn new(tenant_id: String, canonical_repo_identity: String) -> Self {
        Self {
            tenant_id,
            canonical_repo_identity,
        }
    }
}

pub fn env_enables_hosted_review() -> bool {
    std::env::var("COVEN_CODE_HOSTED_REVIEW")
        .map(|value| is_truthy(&value))
        .unwrap_or(false)
}

fn is_truthy(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "off"
    )
}

fn is_false(value: &bool) -> bool {
    !*value
}
