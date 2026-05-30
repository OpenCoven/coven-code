//! Shared access to Coven daemon state under `~/.coven/`.
//!
//! coven-code keeps its own private state under `~/.coven-code/`, but the
//! Coven daemon (`coven`) maintains canonical user-facing state under
//! `~/.coven/` — familiars roster, skills manifests, memory, etc. This
//! module is the read-only bridge: nothing here writes to the daemon's
//! directory, and every loader returns `None` / empty when the daemon is
//! absent so coven-code keeps working standalone.
//!
//! Tier A of the "native Coven" integration. Tier B (daemon IPC over
//! `~/.coven/coven.sock`) lives in [`crate::coven_daemon`].

// Re-export Tier B IPC types for convenience.
pub use crate::coven_daemon::{CreateSessionRequest, DaemonClient, DaemonSession, FamiliarStatus};

use serde::Deserialize;
use std::path::PathBuf;

/// Locate `~/.coven/` if it exists.
///
/// Respects `COVEN_HOME` env var for testability and non-default daemons.
/// Returns `None` when the directory cannot be resolved or does not exist —
/// callers should degrade gracefully.
pub fn coven_home() -> Option<PathBuf> {
    if let Ok(override_path) = std::env::var("COVEN_HOME") {
        if !override_path.is_empty() {
            let p = PathBuf::from(override_path);
            return p.is_dir().then_some(p);
        }
    }
    let p = dirs::home_dir()?.join(".coven");
    p.is_dir().then_some(p)
}

#[cfg(test)]
pub(crate) static COVEN_HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ---------------------------------------------------------------------------
// Familiars (~/.coven/familiars.toml)
// ---------------------------------------------------------------------------

/// Default tool-access tier applied when a familiar omits the `access` field.
///
/// Intentionally restrictive — write/exec power is opt-in by setting
/// `access = "full"` per familiar in `~/.coven/familiars.toml`.
pub const DEFAULT_FAMILIAR_ACCESS: &str = "read-only";

/// One entry in `~/.coven/familiars.toml`.
///
/// Schema mirrors what the daemon serves at `GET /api/v1/familiars`.
#[derive(Debug, Clone, Deserialize)]
pub struct CovenFamiliar {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub pronouns: Option<String>,
    /// Tool-access tier: `"full"`, `"read-only"`, or `"search-only"`.
    /// Absent → [`DEFAULT_FAMILIAR_ACCESS`] (`"read-only"`).
    #[serde(default)]
    pub access: Option<String>,
}

impl CovenFamiliar {
    /// Resolved access tier — the explicit value or [`DEFAULT_FAMILIAR_ACCESS`].
    pub fn resolved_access(&self) -> &str {
        self.access.as_deref().unwrap_or(DEFAULT_FAMILIAR_ACCESS)
    }
}

#[derive(Debug, Deserialize)]
struct FamiliarsFile {
    #[serde(default)]
    familiar: Vec<CovenFamiliar>,
}

/// Load familiars from `~/.coven/familiars.toml`.
/// Returns `None` if the daemon dir, the file, or the parse fails.
pub fn load_familiars() -> Option<Vec<CovenFamiliar>> {
    let path = coven_home()?.join("familiars.toml");
    let raw = std::fs::read_to_string(&path).ok()?;
    let parsed: FamiliarsFile = toml::from_str(&raw).ok()?;
    if parsed.familiar.is_empty() {
        None
    } else {
        Some(parsed.familiar)
    }
}

/// Build a [`crate::config::AgentDefinition`] from a familiar so it can be
/// selected through the same `--agent` / agent-mode plumbing as built-in
/// agents. Returns `(id, def)` keyed on the familiar's lowercase id.
///
/// The familiar's `access` tier flows into [`crate::config::AgentDefinition::access`]
/// so the existing tool-filter pipeline in the CLI is the single source of
/// truth for what tools a familiar can use.
pub fn familiar_to_agent_definition(
    fam: &CovenFamiliar,
) -> (String, crate::config::AgentDefinition) {
    let display = fam.display_name.as_deref().unwrap_or(&fam.id).to_string();
    let emoji = fam.emoji.as_deref().unwrap_or("✨");
    let role = fam.role.as_deref().unwrap_or("Familiar");
    let desc_body = fam
        .description
        .as_deref()
        .unwrap_or("A Coven familiar persona.")
        .to_string();
    let pronouns = fam
        .pronouns
        .as_deref()
        .map(|p| format!(" Pronouns: {p}."))
        .unwrap_or_default();

    let prompt = format!(
        "You are {emoji} {display}, a Coven familiar with the role of {role}.{pronouns}\n\n{desc_body}\n\nStay in character and remain focused on the developer's goals."
    );

    let def = crate::config::AgentDefinition {
        description: Some(format!("{emoji} {role} — {desc_body}")),
        model: None,
        temperature: None,
        prompt: Some(prompt),
        access: fam.resolved_access().to_string(),
        visible: true,
        max_turns: None,
        color: None,
    };
    (fam.id.to_lowercase(), def)
}

/// Return the merged built-in + familiar agent map.
///
/// Built-in agents win on id collision (familiars share lowercase keyspace
/// with `build`/`plan`/`explore`, so collisions are unexpected — but the
/// rule keeps `build` etc. inviolate). Callers extend with user-defined
/// `config.agents` afterwards so user overrides still win.
pub fn default_agents_with_familiars(
) -> std::collections::HashMap<String, crate::config::AgentDefinition> {
    let mut map = crate::config::default_agents();
    if let Some(fams) = load_familiars() {
        for fam in &fams {
            let (id, def) = familiar_to_agent_definition(fam);
            map.entry(id).or_insert(def);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Skills (~/.coven/skills/<id>/metadata.json)
// ---------------------------------------------------------------------------

/// One skill registered in the daemon's `~/.coven/skills/` directory.
///
/// The daemon currently exposes skills as `metadata.json` manifests inside
/// per-skill subdirectories. coven-code cannot *execute* these skills (its
/// SkillTool expects markdown prompt bodies); they are surfaced as
/// awareness so the model knows what's available via the daemon.
#[derive(Debug, Clone, Deserialize)]
pub struct DaemonSkill {
    /// Directory name under `~/.coven/skills/` — the canonical id.
    #[serde(skip)]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Enumerate daemon skills by scanning `~/.coven/skills/<id>/metadata.json`.
/// Returns an empty vec if the daemon dir is absent or the scan fails — never
/// errors out to the caller.
pub fn list_daemon_skills() -> Vec<DaemonSkill> {
    let Some(skills_dir) = coven_home().map(|h| h.join("skills")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&skills_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
        else {
            continue;
        };
        let manifest = path.join("metadata.json");
        let Ok(raw) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let Ok(mut skill) = serde_json::from_str::<DaemonSkill>(&raw) else {
            continue;
        };
        skill.id = id;
        out.push(skill);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // coven_home() reads COVEN_HOME from process env, which is shared across
    // parallel tests in the same binary. Serialize the env-touching tests so
    // they don't clobber each other's overrides.
    struct EnvGuard {
        _tmp: TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("COVEN_HOME");
        }
    }

    fn with_coven_home<F: FnOnce(&std::path::Path)>(setup: F) -> EnvGuard {
        let lock = COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = TempDir::new().unwrap();
        setup(tmp.path());
        std::env::set_var("COVEN_HOME", tmp.path());
        EnvGuard {
            _tmp: tmp,
            _lock: lock,
        }
    }

    #[test]
    fn coven_home_returns_none_when_dir_missing() {
        let _lock = COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("COVEN_HOME", "/nonexistent/path/cc_test_xyz");
        assert!(coven_home().is_none());
        std::env::remove_var("COVEN_HOME");
    }

    #[test]
    fn load_familiars_parses_valid_file() {
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "nova"
display_name = "Nova"
emoji = "👑"
role = "Queen"
description = "Test orchestrator"
pronouns = "she/her"

[[familiar]]
id = "kitty"
display_name = "Kitty"
role = "General Helper"
"#,
            )
            .unwrap();
        });
        let familiars = load_familiars().expect("should parse");
        assert_eq!(familiars.len(), 2);
        assert_eq!(familiars[0].id, "nova");
        assert_eq!(familiars[0].emoji.as_deref(), Some("👑"));
        assert_eq!(familiars[1].id, "kitty");
        assert!(familiars[1].emoji.is_none());
    }

    #[test]
    fn load_familiars_returns_none_on_missing_file() {
        let _g = with_coven_home(|_| {});
        assert!(load_familiars().is_none());
    }

    #[test]
    fn familiar_access_defaults_to_read_only_when_absent() {
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "sage"
display_name = "Sage"
role = "Research"
"#,
            )
            .unwrap();
        });
        let familiars = load_familiars().expect("should parse");
        assert!(familiars[0].access.is_none());
        assert_eq!(familiars[0].resolved_access(), DEFAULT_FAMILIAR_ACCESS);
        assert_eq!(familiars[0].resolved_access(), "read-only");
    }

    #[test]
    fn familiar_access_parses_explicit_tiers() {
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "cody"
access = "full"

[[familiar]]
id = "sage"
access = "read-only"

[[familiar]]
id = "scout"
access = "search-only"
"#,
            )
            .unwrap();
        });
        let familiars = load_familiars().expect("should parse");
        assert_eq!(familiars[0].resolved_access(), "full");
        assert_eq!(familiars[1].resolved_access(), "read-only");
        assert_eq!(familiars[2].resolved_access(), "search-only");
    }

    #[test]
    fn familiar_to_agent_definition_threads_access_tier() {
        let fam = CovenFamiliar {
            id: "Cody".to_string(),
            display_name: Some("Cody".to_string()),
            emoji: Some("⚡".to_string()),
            role: Some("Code".to_string()),
            description: Some("Builds and ships.".to_string()),
            pronouns: None,
            access: Some("full".to_string()),
        };
        let (id, def) = familiar_to_agent_definition(&fam);
        assert_eq!(id, "cody", "id should be lowercased for map keys");
        assert_eq!(def.access, "full");
        assert!(def.visible);
        let prompt = def.prompt.as_deref().unwrap_or("");
        assert!(prompt.contains("Cody"));
        assert!(prompt.contains("Code"));
    }

    #[test]
    fn familiar_to_agent_definition_defaults_to_read_only() {
        let fam = CovenFamiliar {
            id: "sage".to_string(),
            display_name: None,
            emoji: None,
            role: None,
            description: None,
            pronouns: None,
            access: None,
        };
        let (_id, def) = familiar_to_agent_definition(&fam);
        assert_eq!(def.access, "read-only");
    }

    #[test]
    fn default_agents_with_familiars_merges_without_clobbering_builtins() {
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "cody"
display_name = "Cody"
role = "Code"
access = "full"

[[familiar]]
id = "build"  # collides with built-in; built-in must win
display_name = "Imposter"
access = "search-only"
"#,
            )
            .unwrap();
        });
        let merged = default_agents_with_familiars();
        // Built-in `build` is untouched.
        assert_eq!(merged.get("build").map(|d| d.access.as_str()), Some("full"));
        // Familiar `cody` was merged in with its declared access.
        assert_eq!(merged.get("cody").map(|d| d.access.as_str()), Some("full"));
    }

    #[test]
    fn list_daemon_skills_scans_metadata_files() {
        let _g = with_coven_home(|home| {
            let skill_dir = home.join("skills").join("opencoven-design");
            fs::create_dir_all(&skill_dir).unwrap();
            fs::write(
                skill_dir.join("metadata.json"),
                r#"{"name":"OpenCoven Design","description":"Brand kit","version":"1.0.0","tags":["design","brand"]}"#,
            )
            .unwrap();
            // A dir without metadata.json — should be skipped silently.
            fs::create_dir_all(home.join("skills").join("orphan")).unwrap();
        });
        let skills = list_daemon_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "opencoven-design");
        assert_eq!(skills[0].name.as_deref(), Some("OpenCoven Design"));
        assert_eq!(skills[0].tags, vec!["design", "brand"]);
    }
}
