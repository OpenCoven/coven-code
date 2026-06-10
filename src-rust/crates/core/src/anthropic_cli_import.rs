//! Import existing Anthropic OAuth credentials from an external CLI's
//! credential store into a Coven Code account profile.
//!
//! Subscription / OAuth users who have already logged in with another
//! first-party Anthropic CLI shouldn't have to re-authenticate inside
//! Coven Code. This module discovers credentials produced by:
//!
//!   - **Claude Code** — `~/.claude/.credentials.json`, or (on macOS) the
//!     login Keychain item `Claude Code-credentials`.
//!   - **The `ant` CLI** — `$ANTHROPIC_CONFIG_DIR/credentials/<profile>.json`,
//!     defaulting to `~/.config/anthropic/credentials/`.
//!
//! Parsing is intentionally lenient (accepts both camelCase and snake_case,
//! arrays or space-separated scope strings) so it survives minor format
//! differences across CLI versions. Discovered credentials are persisted as a
//! normal Coven Code account profile via [`OAuthTokens::save_and_register`],
//! after which the existing Anthropic auth-resolution path picks them up.

use crate::oauth::{OAuthTokens, CLAUDE_AI_INFERENCE_SCOPE};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Which external CLI a credential was imported from (used in status messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSource {
    ClaudeCode,
    AntCli,
}

impl ImportSource {
    pub fn label(self) -> &'static str {
        match self {
            ImportSource::ClaudeCode => "Claude Code",
            ImportSource::AntCli => "Anthropic CLI",
        }
    }
}

/// A credential located on disk plus the CLI it came from.
pub struct DiscoveredCredential {
    pub tokens: OAuthTokens,
    pub source: ImportSource,
}

// ---------------------------------------------------------------------------
// Candidate locations
// ---------------------------------------------------------------------------

fn claude_code_credentials_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".claude").join(".credentials.json"))
}

fn ant_credentials_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ANTHROPIC_CONFIG_DIR") {
        let d = PathBuf::from(dir.trim());
        if !d.as_os_str().is_empty() {
            return Some(d.join("credentials"));
        }
    }
    Some(dirs::config_dir()?.join("anthropic").join("credentials"))
}

/// On macOS, Claude Code stores its credentials in the login Keychain rather
/// than a flat file. Read the JSON blob back out via `security`.
#[cfg(target_os = "macos")]
fn claude_code_keychain_json() -> Option<String> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// ---------------------------------------------------------------------------
// Lenient field extraction
// ---------------------------------------------------------------------------

fn first_string(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn first_i64(v: &Value, keys: &[&str]) -> Option<i64> {
    for k in keys {
        match v.get(*k) {
            Some(Value::Number(n)) => {
                if let Some(i) = n.as_i64() {
                    return Some(i);
                }
                if let Some(f) = n.as_f64() {
                    return Some(f as i64);
                }
            }
            Some(Value::String(s)) => {
                if let Ok(i) = s.parse::<i64>() {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn first_scopes(v: &Value, keys: &[&str]) -> Vec<String> {
    for k in keys {
        match v.get(*k) {
            Some(Value::Array(arr)) => {
                return arr
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
            }
            Some(Value::String(s)) => {
                return s.split_whitespace().map(String::from).collect();
            }
            _ => {}
        }
    }
    Vec::new()
}

/// Parse an object that directly holds the OAuth fields (camelCase or
/// snake_case). Returns `None` when there's no usable access token.
fn parse_oauth_object(obj: &Value) -> Option<OAuthTokens> {
    let access_token = first_string(obj, &["accessToken", "access_token"])?;
    let refresh_token = first_string(obj, &["refreshToken", "refresh_token"]);
    let expires_at_ms = first_i64(obj, &["expiresAt", "expires_at", "expires_at_ms"]);
    let mut scopes = first_scopes(obj, &["scopes", "scope"]);
    if scopes.is_empty() {
        // A subscription login is inference-capable; assume the inference
        // scope so the credential is treated as a Bearer token downstream.
        scopes.push(CLAUDE_AI_INFERENCE_SCOPE.to_string());
    }
    let subscription_type = first_string(obj, &["subscriptionType", "subscription_type"]);
    let email = first_string(obj, &["email", "emailAddress", "email_address"]);

    Some(OAuthTokens {
        access_token,
        refresh_token,
        expires_at_ms,
        scopes,
        subscription_type,
        email,
        ..Default::default()
    })
}

/// Parse a Claude Code credentials document (`{ "claudeAiOauth": { … } }`),
/// falling back to a bare OAuth object.
pub fn parse_claude_code(json: &str) -> Option<OAuthTokens> {
    let v: Value = serde_json::from_str(json).ok()?;
    if let Some(inner) = v.get("claudeAiOauth") {
        if let Some(t) = parse_oauth_object(inner) {
            return Some(t);
        }
    }
    parse_oauth_object(&v)
}

/// Parse an `ant` CLI credentials document. The OAuth fields may sit at the top
/// level or under a wrapper key, so try a few shapes.
pub fn parse_ant(json: &str) -> Option<OAuthTokens> {
    let v: Value = serde_json::from_str(json).ok()?;
    for key in ["oauth", "credentials", "claudeAiOauth", "token", "tokens"] {
        if let Some(inner) = v.get(key) {
            if let Some(t) = parse_oauth_object(inner) {
                return Some(t);
            }
        }
    }
    parse_oauth_object(&v)
}

fn read_first_ant_credential(dir: &Path) -> Option<OAuthTokens> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    files.sort();
    // Prefer a profile literally named `default.json`.
    let default_first = files
        .iter()
        .position(|p| p.file_name().and_then(|n| n.to_str()) == Some("default.json"));
    if let Some(idx) = default_first {
        files.swap(0, idx);
    }
    for path in files {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(t) = parse_ant(&text) {
                if !t.access_token.is_empty() {
                    return Some(t);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Discovery + import
// ---------------------------------------------------------------------------

/// Locate an external Anthropic OAuth credential on this machine, trying Claude
/// Code first (file, then macOS Keychain) and then the `ant` CLI.
pub fn discover() -> Option<DiscoveredCredential> {
    if let Some(path) = claude_code_credentials_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(tokens) = parse_claude_code(&text) {
                if !tokens.access_token.is_empty() {
                    return Some(DiscoveredCredential {
                        tokens,
                        source: ImportSource::ClaudeCode,
                    });
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(text) = claude_code_keychain_json() {
        if let Some(tokens) = parse_claude_code(&text) {
            if !tokens.access_token.is_empty() {
                return Some(DiscoveredCredential {
                    tokens,
                    source: ImportSource::ClaudeCode,
                });
            }
        }
    }

    if let Some(dir) = ant_credentials_dir() {
        if let Some(tokens) = read_first_ant_credential(&dir) {
            return Some(DiscoveredCredential {
                tokens,
                source: ImportSource::AntCli,
            });
        }
    }

    None
}

/// Discover and persist an external Anthropic credential as a Coven Code
/// account profile. Returns the source and the new/updated profile id.
pub async fn import() -> anyhow::Result<(ImportSource, String)> {
    let found = discover().ok_or_else(|| {
        anyhow::anyhow!(
            "No Anthropic CLI credentials found. Looked for Claude Code \
             (~/.claude/.credentials.json{}) and the ant CLI \
             (~/.config/anthropic/credentials/). Sign in with `claude` or \
             `ant auth login` first, or use an API key.",
            if cfg!(target_os = "macos") {
                " / Keychain"
            } else {
                ""
            }
        )
    })?;
    let label = found.source.label().to_string();
    let profile_id = found.tokens.save_and_register(Some(&label)).await?;
    Ok((found.source, profile_id))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_code_nested_oauth() {
        let json = r#"{
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-abc",
                "refreshToken": "sk-ant-ort01-def",
                "expiresAt": 1799999999000,
                "scopes": ["user:inference", "user:profile"],
                "subscriptionType": "max"
            }
        }"#;
        let t = parse_claude_code(json).expect("parsed");
        assert_eq!(t.access_token, "sk-ant-oat01-abc");
        assert_eq!(t.refresh_token.as_deref(), Some("sk-ant-ort01-def"));
        assert_eq!(t.expires_at_ms, Some(1799999999000));
        assert!(t.uses_bearer_auth(), "inference scope -> bearer");
        assert_eq!(t.subscription_type.as_deref(), Some("max"));
    }

    #[test]
    fn parses_ant_snake_case_with_string_scope() {
        let json = r#"{
            "access_token": "sk-ant-oat01-xyz",
            "refresh_token": "sk-ant-ort01-uvw",
            "expires_at": "1788888888000",
            "scope": "user:inference user:profile"
        }"#;
        let t = parse_ant(json).expect("parsed");
        assert_eq!(t.access_token, "sk-ant-oat01-xyz");
        assert_eq!(t.refresh_token.as_deref(), Some("sk-ant-ort01-uvw"));
        assert_eq!(t.expires_at_ms, Some(1788888888000));
        assert!(t.uses_bearer_auth());
    }

    #[test]
    fn parses_ant_wrapped_under_oauth_key() {
        let json = r#"{ "oauth": { "accessToken": "tok-1" } }"#;
        let t = parse_ant(json).expect("parsed");
        assert_eq!(t.access_token, "tok-1");
        // No scopes given -> default inference scope assumed.
        assert!(t.uses_bearer_auth());
    }

    #[test]
    fn missing_access_token_yields_none() {
        assert!(parse_claude_code(r#"{ "claudeAiOauth": { "refreshToken": "r" } }"#).is_none());
        assert!(parse_ant(r#"{ "foo": "bar" }"#).is_none());
        assert!(parse_ant("not json").is_none());
    }

    #[test]
    fn read_first_ant_credential_prefers_default_profile() {
        let dir = std::env::temp_dir().join(format!(
            "coven-ant-import-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("alpha.json"),
            r#"{"access_token":"from-alpha"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("default.json"),
            r#"{"access_token":"from-default"}"#,
        )
        .unwrap();

        let t = read_first_ant_credential(&dir).expect("found");
        assert_eq!(t.access_token, "from-default");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
