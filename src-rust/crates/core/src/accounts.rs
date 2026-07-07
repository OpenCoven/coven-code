//! Multi-account credential management.
//!
//! Stores named profiles per provider (anthropic, codex) on disk so users can
//! switch between Pro/Max/work/personal accounts without re-logging-in.
//!
//! Design borrows from two prior arts:
//!
//!   * **codexmaxx** (kitze/codexmaxx) — named per-account snapshots stored on
//!     disk, identity derived from JWT payload (email / account_id), explicit
//!     "import current external login" flow.
//!   * **opencode** — single tagged-union JSON file, chmod 0600, symmetric
//!     `list / login / logout / switch` commands across providers.
//!
//! Layout:
//!
//! ```text
//! ~/.coven-code/
//!   accounts.json                              # registry (this module)
//!   accounts/
//!     anthropic/<profile-id>/oauth_tokens.json
//!     codex/<profile-id>/codex_tokens.json
//!   oauth_tokens.json                          # legacy (auto-migrated)
//!   codex_tokens.json                          # legacy (auto-migrated)
//! ```
//!
//! The registry holds metadata (label, email, account-id, timestamps, active
//! pointer per provider). The per-account credential files keep their existing
//! schemas so the rest of the codebase doesn't change shape.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Identifier for a credential provider that supports multi-account.
pub const PROVIDER_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_CODEX: &str = "codex";

/// Metadata recorded for a single stored profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountProfile {
    /// Slug used as the directory name and CLI identifier.
    pub id: String,
    /// Optional human-friendly label (e.g. "work", "personal").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Email extracted from the JWT id_token (when available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Provider-side account identifier (account_id / account_uuid).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Organization UUID (Anthropic only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_uuid: Option<String>,
    /// Plan / subscription tier (Pro, Max, …) when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_tier: Option<String>,
    /// ISO-8601 timestamp when this profile was first added.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added_at: Option<String>,
    /// ISO-8601 timestamp of the last `switch_to(...)` call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_selected_at: Option<String>,
}

impl AccountProfile {
    /// Best-effort display name for menus: label > email > id.
    pub fn display_name(&self) -> String {
        self.label
            .clone()
            .or_else(|| self.email.clone())
            .unwrap_or_else(|| self.id.clone())
    }
}

/// Per-provider section of the registry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderAccounts {
    /// Profile id of the currently-active account for this provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// All stored profiles, keyed by id.
    #[serde(default)]
    pub profiles: BTreeMap<String, AccountProfile>,
}

/// On-disk shape of `~/.coven-code/accounts.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountRegistry {
    /// Schema version (current: 1).
    #[serde(default = "default_version")]
    pub version: u32,
    /// One entry per credential provider.
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderAccounts>,
}

fn default_version() -> u32 {
    1
}

impl AccountRegistry {
    /// Path to `~/.coven-code/accounts.json`.
    pub fn path() -> PathBuf {
        claurst_dir().join("accounts.json")
    }

    /// Load the registry. Returns an empty registry if the file is missing or
    /// malformed.
    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(reg) = serde_json::from_str::<AccountRegistry>(&data) {
                return reg;
            }
        }
        AccountRegistry::default()
    }

    /// Persist the registry to disk. Best-effort but propagates I/O errors.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        set_user_only_perms(&path);
        Ok(())
    }

    /// Get the active profile id for a provider.
    pub fn active(&self, provider: &str) -> Option<&str> {
        self.providers
            .get(provider)
            .and_then(|p| p.active.as_deref())
    }

    /// Get the active profile metadata, if any.
    pub fn active_profile(&self, provider: &str) -> Option<&AccountProfile> {
        let p = self.providers.get(provider)?;
        let id = p.active.as_ref()?;
        p.profiles.get(id)
    }

    /// List all profiles for a provider (sorted by id).
    pub fn list(&self, provider: &str) -> Vec<AccountProfile> {
        self.providers
            .get(provider)
            .map(|p| p.profiles.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Lookup a profile by id within a provider.
    pub fn get(&self, provider: &str, id: &str) -> Option<&AccountProfile> {
        self.providers.get(provider)?.profiles.get(id)
    }

    /// Insert or update a profile, optionally setting it active.
    pub fn upsert(
        &mut self,
        provider: &str,
        mut profile: AccountProfile,
        make_active: bool,
    ) -> anyhow::Result<()> {
        if profile.added_at.is_none() {
            profile.added_at = Some(now_iso());
        }
        let section = self.providers.entry(provider.to_string()).or_default();
        section.profiles.insert(profile.id.clone(), profile.clone());
        if make_active {
            section.active = Some(profile.id.clone());
            if let Some(stored) = section.profiles.get_mut(&profile.id) {
                stored.last_selected_at = Some(now_iso());
            }
        }
        self.save()
    }

    /// Switch the active profile for a provider. Returns `Err` if the id does
    /// not exist.
    pub fn switch_to(&mut self, provider: &str, id: &str) -> anyhow::Result<()> {
        let section = self
            .providers
            .get_mut(provider)
            .ok_or_else(|| anyhow::anyhow!("No accounts stored for {provider}"))?;
        if !section.profiles.contains_key(id) {
            anyhow::bail!("Account '{}' not found for {}", id, provider);
        }
        section.active = Some(id.to_string());
        if let Some(p) = section.profiles.get_mut(id) {
            p.last_selected_at = Some(now_iso());
        }
        self.save()
    }

    /// Remove a profile (and its credential directory). If it was active,
    /// clears the active pointer.
    pub fn remove(&mut self, provider: &str, id: &str) -> anyhow::Result<()> {
        if let Some(section) = self.providers.get_mut(provider) {
            section.profiles.remove(id);
            if section.active.as_deref() == Some(id) {
                section.active = None;
            }
        }
        // Remove the per-account credential dir.
        let dir = account_dir(provider, id);
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        self.save()
    }
}

/// Slugify an arbitrary string into a safe profile id. Lowercases, replaces
/// non-`[a-z0-9_-]` with `-`, trims dashes/underscores from edges, falls back
/// to "account" if the result is empty.
pub fn slugify_profile_id(raw: &str) -> String {
    let lowered = raw.trim().to_lowercase();
    let mapped: String = lowered
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = mapped
        .trim_matches(|c: char| c == '-' || c == '_')
        .to_string();
    if trimmed.is_empty() {
        "account".to_string()
    } else {
        trimmed
    }
}

/// If the requested id already exists, suffix with -2, -3, … until free.
pub fn ensure_unique_profile_id(registry: &AccountRegistry, provider: &str, base: &str) -> String {
    let base = slugify_profile_id(base);
    if registry.get(provider, &base).is_none() {
        return base;
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{}-{}", base, n);
        if registry.get(provider, &candidate).is_none() {
            return candidate;
        }
        n += 1;
    }
}

/// `~/.coven-code/`.
pub fn claurst_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".coven-code")
}

/// `~/.coven-code/accounts/<provider>/<id>/`.
pub fn account_dir(provider: &str, id: &str) -> PathBuf {
    claurst_dir().join("accounts").join(provider).join(id)
}

/// File where the per-account Anthropic OAuth tokens live.
pub fn anthropic_token_path(profile_id: &str) -> PathBuf {
    account_dir(PROVIDER_ANTHROPIC, profile_id).join("oauth_tokens.json")
}

/// File where the per-account Codex OAuth tokens live.
pub fn codex_token_path(profile_id: &str) -> PathBuf {
    account_dir(PROVIDER_CODEX, profile_id).join("codex_tokens.json")
}

/// Backup directory for the previous live token file (rotated on each switch).
pub fn backup_dir(provider: &str) -> PathBuf {
    claurst_dir()
        .join("accounts")
        .join(provider)
        .join(".backups")
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Tighten permissions on credential files to `0600` (owner read/write only).
///
/// Best-effort and Unix-only: on Windows this is a no-op and access control is
/// left to the filesystem ACLs of the user's home directory. Every file that
/// stores a secret (API keys, OAuth access/refresh tokens) routes through this
/// before it can be read by another local user under a permissive umask.
#[allow(unused_variables)]
pub(crate) fn set_user_only_perms(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

// ---------------------------------------------------------------------------
// Duplicate-profile detection & cleanup
// ---------------------------------------------------------------------------

/// Summary of a duplicate-profile cleanup run.
#[derive(Debug, Clone, Default)]
pub struct DedupeSummary {
    /// Profile ids that were kept (one per distinct credential).
    pub kept: Vec<String>,
    /// Profile ids that were removed as duplicates.
    pub removed: Vec<String>,
}

/// Read the refresh-token identity of a stored Anthropic profile.
///
/// Two profiles whose credential files carry the same refresh token are the
/// same underlying Anthropic account — re-importing an external CLI login used
/// to stack `claude-code-2`, `-3`, … duplicates that all bill one subscription.
/// Access tokens rotate on refresh, so the refresh token (falling back to the
/// access token) is the stable identity. Best-effort: unreadable or malformed
/// files return `None` and are never treated as duplicates of anything.
///
/// This does synchronous disk I/O — call it from command/event handling, never
/// from a render path.
fn anthropic_profile_identity(profile_id: &str) -> Option<String> {
    let raw = std::fs::read_to_string(anthropic_token_path(profile_id)).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    // account_uuid is the strongest identity when present.
    if let Some(uuid) = json.get("account_uuid").and_then(|v| v.as_str()) {
        if !uuid.is_empty() {
            return Some(format!("uuid:{uuid}"));
        }
    }
    let refresh = json.get("refresh_token").and_then(|v| v.as_str());
    let access = json.get("access_token").and_then(|v| v.as_str());
    match (refresh, access) {
        (Some(r), _) if !r.is_empty() => Some(format!("refresh:{r}")),
        (_, Some(a)) if !a.is_empty() => Some(format!("access:{a}")),
        _ => None,
    }
}

/// Millisecond expiry of a stored Anthropic profile's access token (0 when
/// missing/unreadable). Used to pick the freshest credential among duplicates.
fn anthropic_profile_expiry_ms(profile_id: &str) -> u64 {
    std::fs::read_to_string(anthropic_token_path(profile_id))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|json| json.get("expires_at_ms").and_then(|v| v.as_u64()))
        .unwrap_or(0)
}

/// Count how many stored Anthropic profiles are redundant duplicates of
/// another profile (same underlying account). `0` means the registry is clean.
pub fn count_duplicate_anthropic_profiles(registry: &AccountRegistry) -> usize {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for profile in registry.list(PROVIDER_ANTHROPIC) {
        if let Some(identity) = anthropic_profile_identity(&profile.id) {
            *seen.entry(identity).or_insert(0) += 1;
        }
    }
    seen.values().map(|n| n.saturating_sub(1)).sum()
}

/// Collapse duplicate Anthropic profiles down to one profile per distinct
/// underlying account.
///
/// Within each duplicate group the survivor is chosen by (in order): the
/// currently-active profile, then the credential with the latest
/// `expires_at_ms` (freshest token), then lexicographically-smallest id for
/// determinism. The active profile is always its group's survivor, so the
/// active pointer never dangles. Credential directories of removed profiles
/// are deleted.
pub fn dedupe_anthropic_profiles(registry: &mut AccountRegistry) -> anyhow::Result<DedupeSummary> {
    let active = registry.active(PROVIDER_ANTHROPIC).map(|id| id.to_string());
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for profile in registry.list(PROVIDER_ANTHROPIC) {
        if let Some(identity) = anthropic_profile_identity(&profile.id) {
            groups.entry(identity).or_default().push(profile.id);
        }
    }

    let summary = plan_dedupe(groups, active.as_deref(), anthropic_profile_expiry_ms);
    for id in &summary.removed {
        registry.remove(PROVIDER_ANTHROPIC, id)?;
    }
    Ok(summary)
}

/// Pure survivor-selection over identity groups. Extracted from
/// [`dedupe_anthropic_profiles`] so the policy is unit-testable without disk.
fn plan_dedupe(
    groups: BTreeMap<String, Vec<String>>,
    active: Option<&str>,
    expiry_of: impl Fn(&str) -> u64,
) -> DedupeSummary {
    let mut summary = DedupeSummary::default();
    for (_, mut ids) in groups {
        if ids.len() < 2 {
            if let Some(id) = ids.pop() {
                summary.kept.push(id);
            }
            continue;
        }
        ids.sort();
        let survivor = ids
            .iter()
            .find(|id| Some(id.as_str()) == active)
            .cloned()
            .unwrap_or_else(|| {
                // No active profile in this group — keep the freshest token.
                let mut best = ids[0].clone();
                let mut best_expiry = expiry_of(&best);
                for id in &ids[1..] {
                    let expiry = expiry_of(id);
                    if expiry > best_expiry {
                        best = id.clone();
                        best_expiry = expiry;
                    }
                }
                best
            });
        for id in ids {
            if id != survivor {
                summary.removed.push(id);
            }
        }
        summary.kept.push(survivor);
    }
    summary
}

// ---------------------------------------------------------------------------
// JWT identity extraction
// ---------------------------------------------------------------------------

/// Identity fields extracted from an OpenAI/Codex id_token or access_token.
#[derive(Debug, Clone, Default)]
pub struct JwtIdentity {
    pub email: Option<String>,
    pub account_id: Option<String>,
}

/// Decode the payload of a JWT (`header.payload.signature`) and pull out the
/// fields we care about for naming a profile. Tolerates malformed input by
/// returning an empty identity.
pub fn jwt_identity(token: &str) -> JwtIdentity {
    use base64::Engine;

    let mut out = JwtIdentity::default();
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    let Some(payload_b64) = parts.get(1) else {
        return out;
    };

    // JWT payloads are base64url-encoded without padding.
    let mut padded = (*payload_b64).to_string();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    let bytes = match base64::engine::general_purpose::URL_SAFE.decode(padded.as_bytes()) {
        Ok(b) => b,
        Err(_) => return out,
    };
    let json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return out,
    };

    // Direct email claim wins; otherwise look at the OpenAI custom profile claim.
    if let Some(email) = json.get("email").and_then(|v| v.as_str()) {
        out.email = Some(email.to_string());
    } else if let Some(profile) = json
        .get("https://api.openai.com/profile")
        .and_then(|v| v.as_object())
    {
        if let Some(email) = profile.get("email").and_then(|v| v.as_str()) {
            out.email = Some(email.to_string());
        }
    }

    // OpenAI puts account_id under the custom auth claim.
    if let Some(auth) = json
        .get("https://api.openai.com/auth")
        .and_then(|v| v.as_object())
    {
        if let Some(id) = auth.get("account_id").and_then(|v| v.as_str()) {
            out.account_id = Some(id.to_string());
        }
    }

    out
}

/// Derive a short, human-friendly profile id from a JWT identity. Falls back
/// to "account" if nothing useful is in the token.
pub fn id_from_identity(identity: &JwtIdentity) -> String {
    if let Some(email) = &identity.email {
        // Use the local-part of the email (before @) as the slug source.
        let local = email.split('@').next().unwrap_or(email);
        return slugify_profile_id(local);
    }
    if let Some(account_id) = &identity.account_id {
        return slugify_profile_id(account_id);
    }
    "account".to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_strips_punctuation_and_lowercases() {
        assert_eq!(slugify_profile_id("Work Account!"), "work-account");
        assert_eq!(slugify_profile_id("  --weird-- "), "weird");
        assert_eq!(slugify_profile_id(""), "account");
        assert_eq!(slugify_profile_id("kuber@example.com"), "kuber-example-com");
    }

    #[test]
    fn ensure_unique_appends_suffix() {
        let mut reg = AccountRegistry::default();
        let mut section = ProviderAccounts::default();
        section.profiles.insert(
            "work".to_string(),
            AccountProfile {
                id: "work".into(),
                ..Default::default()
            },
        );
        reg.providers.insert(PROVIDER_ANTHROPIC.into(), section);

        let next = ensure_unique_profile_id(&reg, PROVIDER_ANTHROPIC, "work");
        assert_eq!(next, "work-2");
        let fresh = ensure_unique_profile_id(&reg, PROVIDER_ANTHROPIC, "personal");
        assert_eq!(fresh, "personal");
    }

    #[test]
    fn jwt_identity_is_lenient_to_garbage() {
        let identity = jwt_identity("not.a.jwt");
        assert!(identity.email.is_none());
        assert!(identity.account_id.is_none());

        let empty = jwt_identity("");
        assert!(empty.email.is_none());
    }

    #[test]
    fn jwt_identity_pulls_email_and_account_id() {
        use base64::Engine;
        let payload = serde_json::json!({
            "email": "kuber@example.com",
            "https://api.openai.com/auth": {
                "account_id": "acc_abc123"
            }
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_string(&payload).unwrap());
        let token = format!("header.{}.signature", payload_b64);
        let identity = jwt_identity(&token);
        assert_eq!(identity.email.as_deref(), Some("kuber@example.com"));
        assert_eq!(identity.account_id.as_deref(), Some("acc_abc123"));

        assert_eq!(id_from_identity(&identity), "kuber");
    }

    #[test]
    fn account_paths_are_under_claurst_dir() {
        let p = anthropic_token_path("work");
        assert!(p.ends_with("accounts/anthropic/work/oauth_tokens.json"));
        let c = codex_token_path("personal");
        assert!(c.ends_with("accounts/codex/personal/codex_tokens.json"));
    }

    #[test]
    fn account_profile_display_falls_back_through_label_email_id() {
        let mut p = AccountProfile {
            id: "kuber".into(),
            ..Default::default()
        };
        assert_eq!(p.display_name(), "kuber");
        p.email = Some("kuber@example.com".into());
        assert_eq!(p.display_name(), "kuber@example.com");
        p.label = Some("Personal".into());
        assert_eq!(p.display_name(), "Personal");
    }

    #[test]
    fn plan_dedupe_keeps_active_profile_in_its_group() {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        groups.insert(
            "refresh:tok-a".into(),
            vec![
                "claude-code".into(),
                "claude-code-2".into(),
                "claude-code-3".into(),
            ],
        );
        let plan = plan_dedupe(groups, Some("claude-code-2"), |_| 0);
        assert_eq!(plan.kept, vec!["claude-code-2".to_string()]);
        assert_eq!(
            plan.removed,
            vec!["claude-code".to_string(), "claude-code-3".to_string()]
        );
    }

    #[test]
    fn plan_dedupe_prefers_freshest_token_when_active_elsewhere() {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        groups.insert(
            "refresh:tok-a".into(),
            vec!["old".into(), "fresh".into(), "stale".into()],
        );
        let plan = plan_dedupe(groups, None, |id| match id {
            "fresh" => 300,
            "old" => 200,
            _ => 100,
        });
        assert_eq!(plan.kept, vec!["fresh".to_string()]);
        assert_eq!(plan.removed, vec!["old".to_string(), "stale".to_string()]);
    }

    #[test]
    fn plan_dedupe_leaves_distinct_accounts_alone() {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        groups.insert("refresh:tok-a".into(), vec!["work".into()]);
        groups.insert("refresh:tok-b".into(), vec!["personal".into()]);
        let plan = plan_dedupe(groups, Some("work"), |_| 0);
        assert!(plan.removed.is_empty());
        let mut kept = plan.kept.clone();
        kept.sort();
        assert_eq!(kept, vec!["personal".to_string(), "work".to_string()]);
    }

    #[test]
    fn plan_dedupe_ties_break_lexicographically() {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        groups.insert(
            "refresh:tok-a".into(),
            vec!["b-profile".into(), "a-profile".into()],
        );
        let plan = plan_dedupe(groups, None, |_| 42);
        assert_eq!(plan.kept, vec!["a-profile".to_string()]);
        assert_eq!(plan.removed, vec!["b-profile".to_string()]);
    }
}
