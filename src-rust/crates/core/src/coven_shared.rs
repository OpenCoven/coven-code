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
//! `~/.coven/coven.sock`) is not implemented here.

use std::path::PathBuf;
use serde::Deserialize;

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

// ---------------------------------------------------------------------------
// Familiars (~/.coven/familiars.toml)
// ---------------------------------------------------------------------------

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
        let Some(id) = path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string()) else {
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
    use std::sync::Mutex;
    use tempfile::TempDir;

    // coven_home() reads COVEN_HOME from process env, which is shared across
    // parallel tests in the same binary. Serialize the env-touching tests so
    // they don't clobber each other's overrides.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = TempDir::new().unwrap();
        setup(tmp.path());
        std::env::set_var("COVEN_HOME", tmp.path());
        EnvGuard { _tmp: tmp, _lock: lock }
    }

    #[test]
    fn coven_home_returns_none_when_dir_missing() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
