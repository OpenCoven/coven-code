use serde::{Deserialize, Serialize};

const DEFAULT_PROVIDER: &str = "github";
const DEFAULT_HOST: &str = "github.com";

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedReviewConfig {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_user_memory: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_managed_rules: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_write_tools: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_mcp_servers: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_plugins: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_auto_memory_persistence: bool,
    #[serde(default, skip_serializing_if = "MemorySourceTrust::is_unknown")]
    pub memory_source_trust: MemorySourceTrust,
    #[serde(
        default = "default_memory_trust_threshold",
        skip_serializing_if = "is_default_memory_trust_threshold"
    )]
    pub memory_trust_threshold: MemorySourceTrust,
}

impl Default for HostedReviewConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_user_memory: false,
            allow_managed_rules: false,
            allow_write_tools: false,
            allow_mcp_servers: false,
            allow_plugins: false,
            allow_auto_memory_persistence: false,
            memory_source_trust: MemorySourceTrust::Unknown,
            memory_trust_threshold: default_memory_trust_threshold(),
        }
    }
}

impl HostedReviewConfig {
    pub fn is_default(&self) -> bool {
        !self.enabled
            && !self.allow_user_memory
            && !self.allow_managed_rules
            && !self.allow_write_tools
            && !self.allow_mcp_servers
            && !self.allow_plugins
            && !self.allow_auto_memory_persistence
            && self.memory_source_trust == MemorySourceTrust::Unknown
            && self.memory_trust_threshold == default_memory_trust_threshold()
    }

    pub fn memory_source_trust(&self) -> MemorySourceTrust {
        self.memory_source_trust
    }

    pub fn memory_trust_threshold(&self) -> MemorySourceTrust {
        self.memory_trust_threshold
    }

    pub fn allows_auto_memory_persistence(&self) -> bool {
        self.allow_auto_memory_persistence
            && self
                .memory_source_trust
                .meets_threshold(self.memory_trust_threshold)
    }
}

/// Trust classification for the source that produced or approved memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MemorySourceTrust {
    SystemPolicy,
    MaintainerApproved,
    DefaultBranchCode,
    ContributorInput,
    ForkInput,
    ModelInferred,
    #[default]
    Unknown,
}

impl MemorySourceTrust {
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    pub fn meets_threshold(self, threshold: Self) -> bool {
        self.rank() >= threshold.rank()
    }

    pub fn capped_at(self, max: Self) -> Self {
        if self.rank() > max.rank() {
            max
        } else {
            self
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::ForkInput => 10,
            Self::ContributorInput => 20,
            Self::ModelInferred => 30,
            Self::DefaultBranchCode => 60,
            Self::MaintainerApproved => 80,
            Self::SystemPolicy => 100,
        }
    }
}

fn default_memory_trust_threshold() -> MemorySourceTrust {
    MemorySourceTrust::MaintainerApproved
}

fn is_default_memory_trust_threshold(value: &MemorySourceTrust) -> bool {
    *value == default_memory_trust_threshold()
}

/// Canonical repository identity supplied by the hosted control plane or
/// derived from a git remote for local diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalRepoIdentity {
    pub provider: String,
    pub host: String,
    pub owner: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
}

impl CanonicalRepoIdentity {
    pub fn github(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            provider: DEFAULT_PROVIDER.to_string(),
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
            repo_id: None,
            node_id: None,
            default_branch: None,
        }
    }

    /// Reject identities with empty or whitespace-only components. An empty
    /// component would collapse the derived namespace across repositories.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("provider", &self.provider),
            ("host", &self.host),
            ("owner", &self.owner),
            ("name", &self.name),
        ] {
            if value.trim().is_empty() {
                return Err(format!("canonical repo identity has empty {field}"));
            }
        }
        if let Some(repo_id) = &self.repo_id {
            if repo_id.trim().is_empty() {
                return Err("canonical repo identity has empty repo_id".to_string());
            }
        }
        Ok(())
    }

    pub fn with_repo_id(mut self, repo_id: impl Into<String>) -> Self {
        self.repo_id = Some(repo_id.into());
        self
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn stable_repo_key(&self) -> String {
        self.repo_id
            .as_deref()
            .map(safe_component)
            .unwrap_or_else(|| {
                safe_component(&format!(
                    "{}_{}_{}_{}",
                    self.provider, self.host, self.owner, self.name
                ))
            })
    }

    pub fn canonical_string(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.provider, self.host, self.owner, self.name
        )
    }

    pub fn from_git_remote_url(remote_url: &str) -> Option<Self> {
        parse_url_remote(remote_url).or_else(|| parse_scp_remote(remote_url))
    }
}

/// Hosted memory domain. Domains are intentionally part of durable hosted
/// storage keys so branch, PR, and security-private memory cannot collide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case", tag = "type", content = "value")]
pub enum MemoryDomain {
    #[default]
    DefaultBranch,
    Branch(String),
    Release(String),
    PullRequest(u64),
    SecurityPrivate,
}

impl MemoryDomain {
    pub fn path_component(&self) -> String {
        match self {
            Self::DefaultBranch => "default-branch".to_string(),
            Self::Branch(name) => format!("branch-{}", safe_component(name)),
            Self::Release(name) => format!("release-{}", safe_component(name)),
            Self::PullRequest(number) => format!("pr-{number}"),
            Self::SecurityPrivate => "security-private".to_string(),
        }
    }

    pub fn can_load_in_public_review(&self, allow_security_private: bool) -> bool {
        !matches!(self, Self::SecurityPrivate) || allow_security_private
    }
}

/// Tenant/repository identity required before hosted mode may persist
/// durable memory or transcript artifacts into hosted namespaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedReviewScope {
    pub tenant_id: String,
    pub installation_id: String,
    pub repo_id: String,
    pub repo_full_name: String,
    pub canonical_repo_identity: String,
    pub memory_domain: MemoryDomain,
}

impl HostedReviewScope {
    pub fn new(
        tenant_id: String,
        installation_id: String,
        repo_id: String,
        repo_full_name: String,
    ) -> Self {
        let canonical_repo_identity = format!("{DEFAULT_PROVIDER}/{DEFAULT_HOST}/{repo_full_name}");
        Self {
            tenant_id,
            installation_id,
            repo_id,
            repo_full_name,
            canonical_repo_identity,
            memory_domain: MemoryDomain::DefaultBranch,
        }
    }

    pub fn from_identity(
        tenant_id: String,
        installation_id: String,
        identity: CanonicalRepoIdentity,
    ) -> Self {
        Self {
            tenant_id,
            installation_id,
            repo_id: identity.stable_repo_key(),
            repo_full_name: identity.full_name(),
            canonical_repo_identity: identity.canonical_string(),
            memory_domain: MemoryDomain::DefaultBranch,
        }
    }

    pub fn with_domain(mut self, memory_domain: MemoryDomain) -> Self {
        self.memory_domain = memory_domain;
        self
    }

    /// Full-scope validation: every identity component must be non-empty
    /// after trimming. Hosted namespaces derived from a scope with an empty
    /// component would collapse tenant/installation/repo isolation, so all
    /// hosted persistence and sync surfaces must call this before keying
    /// anything durable on the scope.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("tenant_id", &self.tenant_id),
            ("installation_id", &self.installation_id),
            ("repo_id", &self.repo_id),
            ("repo_full_name", &self.repo_full_name),
            ("canonical_repo_identity", &self.canonical_repo_identity),
        ] {
            if value.trim().is_empty() {
                return Err(format!(
                    "hosted review scope has empty {field}; refusing to derive a hosted namespace"
                ));
            }
        }
        Ok(())
    }

    pub fn tenant_component(&self) -> String {
        safe_component(&self.tenant_id)
    }

    pub fn installation_component(&self) -> String {
        safe_component(&self.installation_id)
    }

    pub fn repo_component(&self) -> String {
        safe_component(&self.repo_id)
    }

    pub fn domain_component(&self) -> String {
        self.memory_domain.path_component()
    }
}

pub fn hosted_project_id(scope: &HostedReviewScope) -> String {
    format!(
        "hosted-tenant-{}-installation-{}-repo-{}",
        scope.tenant_component(),
        scope.installation_component(),
        scope.repo_component()
    )
}

pub fn hosted_team_memory_repo_key(scope: &HostedReviewScope) -> String {
    format!(
        "tenants/{}/installations/{}/repos/{}/domains/{}",
        scope.tenant_component(),
        scope.installation_component(),
        scope.repo_component(),
        scope.domain_component()
    )
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

fn parse_url_remote(remote_url: &str) -> Option<CanonicalRepoIdentity> {
    let url = url::Url::parse(remote_url).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let mut segments = url.path_segments()?;
    let owner = segments.next()?.to_string();
    let name = segments.next()?.trim_end_matches(".git").to_string();
    if owner.is_empty() || name.is_empty() {
        return None;
    }

    Some(CanonicalRepoIdentity::github(host, owner, name))
}

fn parse_scp_remote(remote_url: &str) -> Option<CanonicalRepoIdentity> {
    let (host_part, path_part) = remote_url.split_once(':')?;
    let host = host_part
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(host_part)
        .to_ascii_lowercase();
    let mut pieces = path_part.split('/');
    let owner = pieces.next()?.to_string();
    let name = pieces.next()?.trim_end_matches(".git").to_string();
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }

    Some(CanonicalRepoIdentity::github(host, owner, name))
}

fn safe_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_') {
            out.push(*byte as char);
        } else {
            out.push('~');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0F) as usize] as char);
        }
    }
    if out.is_empty() {
        "~".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_project_id_uses_installation_and_repo_id() {
        let scope = HostedReviewScope::new(
            "tenant-a".to_string(),
            "install-1".to_string(),
            "repo-99".to_string(),
            "OpenCoven/coven-code".to_string(),
        );

        assert_eq!(
            hosted_project_id(&scope),
            "hosted-tenant-tenant-a-installation-install-1-repo-repo-99"
        );
    }

    #[test]
    fn hosted_team_memory_key_includes_domain() {
        let scope = HostedReviewScope::new(
            "tenant-a".to_string(),
            "install-1".to_string(),
            "repo-99".to_string(),
            "OpenCoven/coven-code".to_string(),
        )
        .with_domain(MemoryDomain::PullRequest(42));

        assert_eq!(
            hosted_team_memory_repo_key(&scope),
            "tenants/tenant-a/installations/install-1/repos/repo-99/domains/pr-42"
        );
    }

    #[test]
    fn hosted_scope_components_encode_traversal_and_separators() {
        let scope = HostedReviewScope::new(
            "..".to_string(),
            "install/one".to_string(),
            r"repo\one".to_string(),
            "OpenCoven/coven-code".to_string(),
        )
        .with_domain(MemoryDomain::Branch("../feature".to_string()));

        assert_eq!(scope.tenant_component(), "~2E~2E");
        assert_eq!(scope.installation_component(), "install~2Fone");
        assert_eq!(scope.repo_component(), "repo~5Cone");
        assert_eq!(scope.domain_component(), "branch-~2E~2E~2Ffeature");
        for component in [
            scope.tenant_component(),
            scope.installation_component(),
            scope.repo_component(),
            scope.domain_component(),
        ] {
            assert!(!matches!(component.as_str(), "." | ".."));
            assert!(!component.contains('/'));
            assert!(!component.contains('\\'));
        }
    }

    #[test]
    fn hosted_scope_component_encoding_is_collision_resistant_for_common_inputs() {
        let encoded = [
            safe_component("a/b"),
            safe_component("a_b"),
            safe_component("a.b"),
            safe_component("a~2Fb"),
            safe_component(""),
        ];
        let unique: std::collections::BTreeSet<_> = encoded.iter().collect();

        assert_eq!(unique.len(), encoded.len());
        assert_eq!(safe_component("."), "~2E");
        assert_eq!(safe_component(".."), "~2E~2E");
        assert_eq!(safe_component(""), "~");
    }

    #[test]
    fn memory_source_trust_enforces_threshold_order() {
        assert!(MemorySourceTrust::MaintainerApproved
            .meets_threshold(MemorySourceTrust::DefaultBranchCode));
        assert!(!MemorySourceTrust::ContributorInput
            .meets_threshold(MemorySourceTrust::MaintainerApproved));
        assert!(!MemorySourceTrust::ForkInput.meets_threshold(MemorySourceTrust::ContributorInput));
    }

    #[test]
    fn hosted_memory_persistence_requires_explicit_trusted_policy() {
        let mut config = HostedReviewConfig {
            enabled: true,
            ..Default::default()
        };
        assert!(!config.allows_auto_memory_persistence());

        config.allow_auto_memory_persistence = true;
        config.memory_source_trust = MemorySourceTrust::ContributorInput;
        assert!(!config.allows_auto_memory_persistence());

        config.memory_source_trust = MemorySourceTrust::MaintainerApproved;
        assert!(config.allows_auto_memory_persistence());
    }

    #[test]
    fn security_private_domain_requires_explicit_public_review_allowance() {
        assert!(!MemoryDomain::SecurityPrivate.can_load_in_public_review(false));
        assert!(MemoryDomain::SecurityPrivate.can_load_in_public_review(true));
        assert!(MemoryDomain::DefaultBranch.can_load_in_public_review(false));
    }

    #[test]
    fn hosted_scope_validate_rejects_empty_components() {
        let valid = HostedReviewScope::new(
            "tenant-a".to_string(),
            "install-1".to_string(),
            "repo-99".to_string(),
            "OpenCoven/coven-code".to_string(),
        );
        assert!(valid.validate().is_ok());

        for (tenant, installation, repo) in [
            ("", "install-1", "repo-99"),
            ("tenant-a", "  ", "repo-99"),
            ("tenant-a", "install-1", ""),
        ] {
            let scope = HostedReviewScope::new(
                tenant.to_string(),
                installation.to_string(),
                repo.to_string(),
                "OpenCoven/coven-code".to_string(),
            );
            let err = scope.validate().unwrap_err();
            assert!(err.contains("empty"), "expected empty-field error: {err}");
        }

        let scope = HostedReviewScope::new(
            "tenant-a".to_string(),
            "install-1".to_string(),
            "repo-99".to_string(),
            "\t".to_string(),
        );
        assert!(scope.validate().is_err());
    }

    #[test]
    fn canonical_identity_validate_rejects_empty_components() {
        let valid = CanonicalRepoIdentity::github("github.com", "OpenCoven", "coven-code");
        assert!(valid.validate().is_ok());

        let empty_owner = CanonicalRepoIdentity::github("github.com", " ", "coven-code");
        assert!(empty_owner.validate().is_err());

        let empty_repo_id = CanonicalRepoIdentity::github("github.com", "OpenCoven", "coven-code")
            .with_repo_id("  ");
        assert!(empty_repo_id.validate().is_err());
    }

    #[test]
    fn two_repos_under_same_installation_have_distinct_namespaces() {
        let first = HostedReviewScope::new(
            "tenant-a".to_string(),
            "install-1".to_string(),
            "repo-1".to_string(),
            "OpenCoven/repo-one".to_string(),
        );
        let second = HostedReviewScope::new(
            "tenant-a".to_string(),
            "install-1".to_string(),
            "repo-2".to_string(),
            "OpenCoven/repo-two".to_string(),
        );

        assert_ne!(hosted_project_id(&first), hosted_project_id(&second));
        assert_ne!(
            hosted_team_memory_repo_key(&first),
            hosted_team_memory_repo_key(&second)
        );
    }

    #[test]
    fn parses_https_git_remote() {
        let identity = CanonicalRepoIdentity::from_git_remote_url(
            "https://github.com/OpenCoven/coven-code.git",
        )
        .unwrap();

        assert_eq!(identity.host, "github.com");
        assert_eq!(identity.owner, "OpenCoven");
        assert_eq!(identity.name, "coven-code");
    }

    #[test]
    fn parses_ssh_git_remote() {
        let identity =
            CanonicalRepoIdentity::from_git_remote_url("git@github.com:OpenCoven/coven-code.git")
                .unwrap();

        assert_eq!(identity.host, "github.com");
        assert_eq!(identity.owner, "OpenCoven");
        assert_eq!(identity.name, "coven-code");
    }
}
