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
pub use crate::coven_daemon::{
    ControlActionResult, CreateSessionRequest, DaemonClient, DaemonError, DaemonHealth,
    DaemonReachability, DaemonSession, EventPage, EventRecord, FamiliarStatus,
};

use serde::{Deserialize, Serialize};
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

/// All recognized access tiers, in canonical form. Anything outside this set
/// is treated as untrusted input by [`resolve_access_tier`] and fails closed
/// to [`DEFAULT_FAMILIAR_ACCESS`].
pub const ACCESS_TIERS: &[&str] = &["full", "read-only", "search-only"];

/// Normalize an access string to a canonical tier without emitting any
/// diagnostic. Trims whitespace and lowercases the input so common surface
/// variants (`" Read-Only "`, `"READ-ONLY"`) round-trip cleanly. Returns
/// `None` for anything that isn't a known tier — callers MUST treat that as
/// untrusted input.
///
/// The returned `&'static str` is one of the canonical entries in
/// [`ACCESS_TIERS`], never the caller's allocation.
pub fn canonicalize_access_tier(input: &str) -> Option<&'static str> {
    match input.trim().to_ascii_lowercase().as_str() {
        "full" => Some("full"),
        "read-only" => Some("read-only"),
        "search-only" => Some("search-only"),
        _ => None,
    }
}

/// Resolve an access string to a canonical tier, failing closed for unknown
/// values. On unknown input, prints a single warning to stderr (so a typo in
/// `~/.coven/familiars.toml` or `settings.json` is visible at the moment the
/// tool filter is applied) and returns [`DEFAULT_FAMILIAR_ACCESS`].
///
/// This is the single entry point the CLI tool-filter pipeline should use —
/// the security model depends on unknown tiers collapsing to the most
/// restrictive option rather than silently passing the full tool list
/// through.
pub fn resolve_access_tier(input: &str) -> &'static str {
    match canonicalize_access_tier(input) {
        Some(canonical) => canonical,
        None => {
            eprintln!(
                "warning: unknown access tier {:?} — falling back to {:?}. Valid tiers: {:?}",
                input, DEFAULT_FAMILIAR_ACCESS, ACCESS_TIERS,
            );
            DEFAULT_FAMILIAR_ACCESS
        }
    }
}

/// Agent names that are strictly disallowed for **any** agent, regardless of
/// source (built-ins, project settings, `~/.coven/familiars.toml`, or runtime
/// creation).
///
/// Compared case-insensitively after trimming. A matching id never enters the
/// runtime agent map, so the Agent tool cannot resolve it and the mode
/// switcher never lists it. Callers go through [`is_disallowed_agent_name`].
pub const DISALLOWED_AGENT_NAMES: &[&str] = &["val", "vale", "valentina"];

/// Whether `name` is a strictly disallowed name for any agent.
///
/// Trims surrounding whitespace and compares case-insensitively against
/// [`DISALLOWED_AGENT_NAMES`]. Used at every point where an agent name can
/// enter the system (merge, load, and explicit creation) so the block holds
/// no matter where the name originates.
pub fn is_disallowed_agent_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    DISALLOWED_AGENT_NAMES.contains(&normalized.as_str())
}

/// Whether `name` is a strictly disallowed name for a **familiar**.
///
/// Familiar identities are declared in `~/.coven/familiars.toml`, not reserved
/// in code. The only blocked values are names banned for every agent.
pub fn is_disallowed_familiar_name(name: &str) -> bool {
    is_disallowed_agent_name(name)
}

/// One entry in `~/.coven/familiars.toml`.
///
/// Schema mirrors what the daemon serves at `GET /api/v1/familiars`.
///
/// `Serialize` is derived with `skip_serializing_if` on every optional field so
/// [`save_familiars`] round-trips a clean, minimal TOML file (no `field = ""`
/// noise) when coven-code owns the roster in standalone mode.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CovenFamiliar {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<String>,
    /// Tool-access tier: `"full"`, `"read-only"`, or `"search-only"`.
    /// Absent → [`DEFAULT_FAMILIAR_ACCESS`] (`"read-only"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
    /// Optional model override for this familiar (e.g. `"claude-opus-4-8"`).
    /// Absent → the familiar inherits the session's default model. This lets a
    /// persona be pinned to a specific model without shadowing it with a
    /// workspace agent `.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl CovenFamiliar {
    /// Resolved access tier — canonicalized to one of [`ACCESS_TIERS`].
    ///
    /// Absent values use [`DEFAULT_FAMILIAR_ACCESS`]. Present-but-unknown
    /// values are normalized silently here (case/whitespace) and otherwise
    /// fall back to [`DEFAULT_FAMILIAR_ACCESS`]; the warning for a true typo
    /// fires at the tool-filter chokepoint so it lands at the moment the
    /// security decision is made instead of at parse time when nothing is
    /// listening.
    pub fn resolved_access(&self) -> &'static str {
        match self.access.as_deref() {
            None => DEFAULT_FAMILIAR_ACCESS,
            Some(raw) => canonicalize_access_tier(raw).unwrap_or(DEFAULT_FAMILIAR_ACCESS),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct FamiliarsFile {
    #[serde(default)]
    familiar: Vec<CovenFamiliar>,
}

/// A `~/.coven/familiars.toml` exists but could not be read or parsed.
///
/// Carries the offending path and a human-readable message so the TUI can
/// surface it instead of the roster silently vanishing.
#[derive(Debug, Clone)]
pub struct FamiliarLoadError {
    pub path: PathBuf,
    pub message: String,
}

impl std::fmt::Display for FamiliarLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for FamiliarLoadError {}

/// Load familiars, distinguishing "nothing to load" from "malformed file".
///
/// - `Ok(None)` — no `~/.coven/` dir or no `familiars.toml` (normal standalone
///   operation; nothing to surface).
/// - `Ok(Some(vec))` — the file parsed. The vec may be empty if the file
///   declared no `[[familiar]]` entries, which is distinct from a missing file.
/// - `Err(_)` — the file exists but could not be read or parsed. Callers that
///   can show UI should surface this; a malformed roster otherwise disappears
///   with no signal to the user.
pub fn load_familiars_result() -> Result<Option<Vec<CovenFamiliar>>, FamiliarLoadError> {
    let Some(home) = coven_home() else {
        return Ok(None);
    };
    let path = home.join("familiars.toml");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(FamiliarLoadError {
                path,
                message: e.to_string(),
            })
        }
    };
    match toml::from_str::<FamiliarsFile>(&raw) {
        Ok(parsed) => Ok(Some(parsed.familiar)),
        Err(e) => Err(FamiliarLoadError {
            path,
            message: e.to_string(),
        }),
    }
}

/// Load familiars from `~/.coven/familiars.toml`.
/// Returns `None` if the daemon dir, the file, or the parse fails.
///
/// This is the graceful-degradation entry point used by the security-critical
/// agent-merge paths: any failure collapses to `None` so a broken roster can
/// never widen the runtime agent map. UI surfaces that want to *report* a
/// malformed file should call [`load_familiars_result`] instead.
pub fn load_familiars() -> Option<Vec<CovenFamiliar>> {
    match load_familiars_result() {
        Ok(Some(fams)) if !fams.is_empty() => Some(fams),
        _ => None,
    }
}

/// Non-fatal warnings about a loaded roster, for surfacing in the UI.
///
/// Covers the failure modes that otherwise happen silently: ids dropped for
/// using a reserved name, duplicate ids (only the first is used), and unknown
/// access tiers (which fail closed to [`DEFAULT_FAMILIAR_ACCESS`]). Returns an
/// empty vec for a clean roster.
pub fn familiar_roster_warnings(fams: &[CovenFamiliar]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for fam in fams {
        let id = fam.id.trim().to_ascii_lowercase();
        if is_disallowed_familiar_name(&id) {
            warnings.push(format!(
                "familiar {:?} uses a reserved name and was skipped",
                fam.id
            ));
            continue;
        }
        if !seen.insert(id) {
            warnings.push(format!(
                "duplicate familiar id {:?} — only the first is used",
                fam.id
            ));
        }
        if let Some(raw) = fam.access.as_deref() {
            if canonicalize_access_tier(raw).is_none() {
                warnings.push(format!(
                    "familiar {:?}: unknown access tier {:?} — using {:?}",
                    fam.id, raw, DEFAULT_FAMILIAR_ACCESS
                ));
            }
        }
    }
    warnings
}

// ---------------------------------------------------------------------------
// Writing the roster (standalone-only)
// ---------------------------------------------------------------------------

/// Resolve the `~/.coven/` path even when the directory does not exist yet.
///
/// Unlike [`coven_home`], this does not require the directory to be present —
/// it is the target for *writing* the roster in standalone mode. Respects
/// `COVEN_HOME`.
pub fn coven_home_path() -> Option<PathBuf> {
    if let Ok(override_path) = std::env::var("COVEN_HOME") {
        if !override_path.is_empty() {
            return Some(PathBuf::from(override_path));
        }
    }
    Some(dirs::home_dir()?.join(".coven"))
}

/// Whether the Coven daemon appears to be running, detected by the presence of
/// its IPC socket at `~/.coven/coven.sock`.
///
/// When the daemon is up it owns `familiars.toml`, so coven-code must treat the
/// roster as read-only. Standalone (no socket) is the only mode in which
/// coven-code writes the file itself.
pub fn daemon_socket_present() -> bool {
    coven_home_path()
        .map(|p| p.join("coven.sock").exists())
        .unwrap_or(false)
}

/// Whether coven-code may write `~/.coven/familiars.toml` directly.
///
/// True only in standalone mode (no daemon socket). When the daemon is running
/// the file is daemon-owned and writes must go through it instead.
pub fn can_write_familiars() -> bool {
    !daemon_socket_present()
}

/// Failure modes for the standalone roster-write API.
#[derive(Debug, Clone)]
pub enum FamiliarWriteError {
    /// The Coven daemon is running and owns `familiars.toml`.
    DaemonOwned,
    /// `~/.coven/` could not be resolved (no home directory).
    NoHome,
    /// The existing roster file is malformed and must be fixed before writing.
    ExistingUnreadable(String),
    /// An id was empty or a reserved/disallowed name.
    InvalidId(String),
    /// A familiar with the target id already exists (create-only paths).
    Duplicate(String),
    /// The target id was not found (edit/remove/rename paths).
    NotFound(String),
    /// Serialization or filesystem I/O failed.
    Io(String),
}

impl std::fmt::Display for FamiliarWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DaemonOwned => write!(
                f,
                "the Coven daemon is running and owns ~/.coven/familiars.toml — \
                 edit familiars through the daemon instead"
            ),
            Self::NoHome => write!(f, "could not resolve ~/.coven/ (no home directory)"),
            Self::ExistingUnreadable(m) => {
                write!(f, "existing familiars.toml is unreadable: {m}")
            }
            Self::InvalidId(id) => write!(f, "invalid familiar id {id:?}"),
            Self::Duplicate(id) => write!(f, "a familiar with id {id:?} already exists"),
            Self::NotFound(id) => write!(f, "no familiar with id {id:?}"),
            Self::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for FamiliarWriteError {}

/// Load the current roster for mutation. Missing file → empty vec; a malformed
/// file is a hard error so a write never silently discards existing entries.
fn load_roster_for_write() -> Result<Vec<CovenFamiliar>, FamiliarWriteError> {
    match load_familiars_result() {
        Ok(Some(fams)) => Ok(fams),
        Ok(None) => Ok(Vec::new()),
        Err(e) => Err(FamiliarWriteError::ExistingUnreadable(e.message)),
    }
}

/// Overwrite `~/.coven/familiars.toml` with `familiars` (standalone only).
///
/// Refuses when the daemon is running ([`daemon_socket_present`]). Creates
/// `~/.coven/` if needed. This is the single write chokepoint the higher-level
/// upsert/remove/rename helpers funnel through.
pub fn save_familiars(familiars: &[CovenFamiliar]) -> Result<(), FamiliarWriteError> {
    if daemon_socket_present() {
        return Err(FamiliarWriteError::DaemonOwned);
    }
    let home = coven_home_path().ok_or(FamiliarWriteError::NoHome)?;
    std::fs::create_dir_all(&home).map_err(|e| FamiliarWriteError::Io(e.to_string()))?;
    let file = FamiliarsFile {
        familiar: familiars.to_vec(),
    };
    let toml = toml::to_string_pretty(&file)
        .map_err(|e| FamiliarWriteError::Io(format!("serialize: {e}")))?;
    std::fs::write(home.join("familiars.toml"), toml)
        .map_err(|e| FamiliarWriteError::Io(e.to_string()))?;
    Ok(())
}

/// Validate an id for a writable familiar: non-empty and not reserved.
fn validate_writable_id(id: &str) -> Result<(), FamiliarWriteError> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(FamiliarWriteError::InvalidId(id.to_string()));
    }
    if is_disallowed_familiar_name(trimmed) {
        return Err(FamiliarWriteError::InvalidId(trimmed.to_string()));
    }
    Ok(())
}

/// Insert a familiar, or replace an existing one with the same (case-insensitive)
/// id. Standalone only. Returns whether an existing entry was replaced.
pub fn upsert_familiar(fam: CovenFamiliar) -> Result<bool, FamiliarWriteError> {
    validate_writable_id(&fam.id)?;
    let mut roster = load_roster_for_write()?;
    let key = fam.id.trim().to_ascii_lowercase();
    let mut replaced = false;
    if let Some(slot) = roster
        .iter_mut()
        .find(|f| f.id.trim().to_ascii_lowercase() == key)
    {
        *slot = fam;
        replaced = true;
    } else {
        roster.push(fam);
    }
    save_familiars(&roster)?;
    Ok(replaced)
}

/// Create a new familiar, erroring if the id already exists. Standalone only.
pub fn create_familiar(fam: CovenFamiliar) -> Result<(), FamiliarWriteError> {
    validate_writable_id(&fam.id)?;
    let roster = load_roster_for_write()?;
    let key = fam.id.trim().to_ascii_lowercase();
    if roster
        .iter()
        .any(|f| f.id.trim().to_ascii_lowercase() == key)
    {
        return Err(FamiliarWriteError::Duplicate(fam.id));
    }
    let mut roster = roster;
    roster.push(fam);
    save_familiars(&roster)
}

/// Remove a familiar by (case-insensitive) id. Standalone only. Returns the
/// removed entry.
pub fn remove_familiar(id: &str) -> Result<CovenFamiliar, FamiliarWriteError> {
    let key = id.trim().to_ascii_lowercase();
    let mut roster = load_roster_for_write()?;
    let Some(pos) = roster
        .iter()
        .position(|f| f.id.trim().to_ascii_lowercase() == key)
    else {
        return Err(FamiliarWriteError::NotFound(id.to_string()));
    };
    let removed = roster.remove(pos);
    save_familiars(&roster)?;
    Ok(removed)
}

/// Rename a familiar's id, preserving all other fields. Standalone only.
pub fn rename_familiar(old_id: &str, new_id: &str) -> Result<(), FamiliarWriteError> {
    validate_writable_id(new_id)?;
    let old_key = old_id.trim().to_ascii_lowercase();
    let new_key = new_id.trim().to_ascii_lowercase();
    let mut roster = load_roster_for_write()?;
    if old_key != new_key
        && roster
            .iter()
            .any(|f| f.id.trim().to_ascii_lowercase() == new_key)
    {
        return Err(FamiliarWriteError::Duplicate(new_id.to_string()));
    }
    let Some(slot) = roster
        .iter_mut()
        .find(|f| f.id.trim().to_ascii_lowercase() == old_key)
    else {
        return Err(FamiliarWriteError::NotFound(old_id.to_string()));
    };
    slot.id = new_id.trim().to_string();
    save_familiars(&roster)
}

/// A couple of example familiars written on first-run bootstrap so a brand-new
/// standalone user has a working roster to switch between and a concrete
/// template to edit. Access tiers are deliberately conservative.
pub fn starter_familiars() -> Vec<CovenFamiliar> {
    vec![
        CovenFamiliar {
            id: "sage".to_string(),
            display_name: Some("Sage".to_string()),
            emoji: Some("\u{1f989}".to_string()), // owl
            role: Some("Guide".to_string()),
            description: Some("A thoughtful pair-programmer who explains as they go.".to_string()),
            pronouns: Some("they/them".to_string()),
            access: Some("read-only".to_string()),
            model: None,
        },
        CovenFamiliar {
            id: "forge".to_string(),
            display_name: Some("Forge".to_string()),
            emoji: Some("\u{1f525}".to_string()), // fire
            role: Some("Builder".to_string()),
            description: Some("A hands-on familiar that writes and runs code.".to_string()),
            pronouns: None,
            access: Some("full".to_string()),
            model: None,
        },
    ]
}

/// Write [`starter_familiars`] to `~/.coven/familiars.toml` for a first-run
/// standalone user. Refuses if the daemon owns the file or a roster already
/// exists (so it never clobbers real entries). Returns the written roster.
pub fn write_starter_roster() -> Result<Vec<CovenFamiliar>, FamiliarWriteError> {
    if daemon_socket_present() {
        return Err(FamiliarWriteError::DaemonOwned);
    }
    if !load_roster_for_write()?.is_empty() {
        return Err(FamiliarWriteError::Duplicate(
            "roster already exists".to_string(),
        ));
    }
    let fams = starter_familiars();
    save_familiars(&fams)?;
    Ok(fams)
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
        // An explicit per-familiar model pins the persona; otherwise `None`
        // means "inherit the session default".
        model: fam.model.clone().filter(|m| !m.trim().is_empty()),
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
/// rule keeps `build` etc. inviolate). When merging settings-defined agents,
/// use [`default_agents_with_familiars_and_config`] so familiar ids keep their
/// trusted access tier instead of being shadowed by project configuration.
pub fn default_agents_with_familiars(
) -> std::collections::HashMap<String, crate::config::AgentDefinition> {
    let mut map = crate::config::default_agents();
    if let Some(fams) = load_familiars() {
        for fam in &fams {
            let (id, def) = familiar_to_agent_definition(fam);
            // Familiars are dynamic; only globally disallowed agent names are
            // filtered here.
            if is_disallowed_familiar_name(&id) {
                continue;
            }
            map.entry(id).or_insert(def);
        }
    }
    map.retain(|id, _| !is_disallowed_agent_name(id));
    map
}

/// Return built-in agents, settings-defined agents, and familiars with the
/// correct security precedence for runtime agent resolution.
///
/// Built-in reserved ids (`build`, `plan`, `explore`) and familiar ids from
/// `~/.coven/familiars.toml` **cannot** be shadowed by project settings.
/// These names carry security-significant access tiers (e.g. `plan` is
/// read-only) and the TUI mode switcher relies on those tiers holding
/// regardless of repository configuration. Non-reserved settings ids are
/// merged as usual.
pub fn default_agents_with_familiars_and_config(
    config_agents: &std::collections::HashMap<String, crate::config::AgentDefinition>,
) -> std::collections::HashMap<String, crate::config::AgentDefinition> {
    let builtins = crate::config::default_agents();
    let mut map = builtins.clone();

    // Only merge in settings-defined agents whose id does not collide with a
    // reserved built-in. This is the security boundary that
    // `resolve_tui_agent_mode` and the Tab cycle in the TUI rely on.
    for (id, def) in config_agents {
        if is_disallowed_agent_name(id) {
            continue;
        }
        if !builtins.contains_key(id) {
            map.insert(id.clone(), def.clone());
        }
    }

    if let Some(fams) = load_familiars() {
        for fam in &fams {
            let (id, def) = familiar_to_agent_definition(fam);
            // Familiars are dynamic; only globally disallowed agent names are
            // filtered here.
            if is_disallowed_familiar_name(&id) {
                continue;
            }
            if !builtins.contains_key(&id) {
                map.insert(id, def);
            }
        }
    }

    // Final guard: a disallowed name must never survive in the runtime map,
    // even if it somehow slipped in as a built-in or via display-name aliasing.
    map.retain(|id, _| !is_disallowed_agent_name(id));

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
id = "atlas"
display_name = "Atlas"
emoji = "👑"
role = "Queen"
description = "Test orchestrator"
pronouns = "she/her"

[[familiar]]
id = "ember"
display_name = "Ember"
role = "General Helper"
"#,
            )
            .unwrap();
        });
        let familiars = load_familiars().expect("should parse");
        assert_eq!(familiars.len(), 2);
        assert_eq!(familiars[0].id, "atlas");
        assert_eq!(familiars[0].emoji.as_deref(), Some("👑"));
        assert_eq!(familiars[1].id, "ember");
        assert!(familiars[1].emoji.is_none());
    }

    #[test]
    fn load_familiars_returns_none_on_missing_file() {
        let _g = with_coven_home(|_| {});
        assert!(load_familiars().is_none());
    }

    #[test]
    fn load_familiars_result_distinguishes_absent_from_malformed() {
        // Missing file → Ok(None), not an error.
        {
            let _g = with_coven_home(|_| {});
            assert!(matches!(load_familiars_result(), Ok(None)));
        }

        // Malformed file → Err carrying the path + message.
        let _g2 = with_coven_home(|home| {
            fs::write(home.join("familiars.toml"), "this is = not [valid toml").unwrap();
        });
        let err = load_familiars_result().expect_err("malformed file should error");
        assert!(err.path.ends_with("familiars.toml"));
        assert!(!err.message.is_empty());
    }

    #[test]
    fn load_familiars_result_reports_empty_roster_as_some() {
        // A file that parses but declares no familiars is distinct from a
        // missing file: Ok(Some(empty)) vs Ok(None).
        let _g = with_coven_home(|home| {
            fs::write(home.join("familiars.toml"), "# no entries\n").unwrap();
        });
        match load_familiars_result() {
            Ok(Some(v)) => assert!(v.is_empty()),
            other => panic!("expected Ok(Some(empty)), got {other:?}"),
        }
        // The graceful wrapper still collapses empty to None.
        assert!(load_familiars().is_none());
    }

    #[test]
    fn familiar_roster_warnings_flags_unknown_tier_and_duplicates() {
        let fams = vec![
            CovenFamiliar {
                id: "willow".to_string(),
                display_name: None,
                emoji: None,
                role: None,
                description: None,
                pronouns: None,
                access: Some("readonly".to_string()), // typo → unknown tier
                model: None,
            },
            CovenFamiliar {
                id: "Willow".to_string(), // duplicate id (case-insensitive)
                display_name: None,
                emoji: None,
                role: None,
                description: None,
                pronouns: None,
                access: Some("full".to_string()),
                model: None,
            },
            CovenFamiliar {
                id: "val".to_string(), // reserved/disallowed name
                display_name: None,
                emoji: None,
                role: None,
                description: None,
                pronouns: None,
                access: None,
                model: None,
            },
        ];
        let warnings = familiar_roster_warnings(&fams);
        assert!(warnings.iter().any(|w| w.contains("unknown access tier")));
        assert!(warnings.iter().any(|w| w.contains("duplicate familiar id")));
        assert!(warnings.iter().any(|w| w.contains("reserved name")));
    }

    #[test]
    fn familiar_roster_warnings_empty_for_clean_roster() {
        let fams = vec![CovenFamiliar {
            id: "atlas".to_string(),
            display_name: Some("Atlas".to_string()),
            emoji: None,
            role: None,
            description: None,
            pronouns: None,
            access: Some("full".to_string()),
            model: None,
        }];
        assert!(familiar_roster_warnings(&fams).is_empty());
    }

    fn writable_familiar(id: &str) -> CovenFamiliar {
        CovenFamiliar {
            id: id.to_string(),
            display_name: None,
            emoji: None,
            role: Some("Helper".to_string()),
            description: None,
            pronouns: None,
            access: Some("read-only".to_string()),
            model: None,
        }
    }

    #[test]
    fn create_and_remove_familiar_round_trip() {
        let _g = with_coven_home(|_| {});
        // No daemon socket in the temp home → writes are allowed.
        assert!(can_write_familiars());

        create_familiar(writable_familiar("nova")).expect("create");
        let roster = load_familiars().expect("roster after create");
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].id, "nova");

        // Duplicate create is rejected.
        let dup = create_familiar(writable_familiar("Nova"));
        assert!(matches!(dup, Err(FamiliarWriteError::Duplicate(_))));

        let removed = remove_familiar("nova").expect("remove");
        assert_eq!(removed.id, "nova");
        assert!(load_familiars().is_none(), "roster empty after remove");
    }

    #[test]
    fn upsert_replaces_existing_and_rename_moves_id() {
        let _g = with_coven_home(|_| {});
        create_familiar(writable_familiar("scout")).expect("create");

        // Upsert with same id (different case) replaces rather than duplicates.
        let mut updated = writable_familiar("Scout");
        updated.access = Some("full".to_string());
        let replaced = upsert_familiar(updated).expect("upsert");
        assert!(replaced);
        let roster = load_familiars().expect("roster");
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].resolved_access(), "full");

        rename_familiar("scout", "ranger").expect("rename");
        let roster = load_familiars().expect("roster");
        assert_eq!(roster[0].id, "ranger");
    }

    #[test]
    fn write_rejects_reserved_and_missing_ids() {
        let _g = with_coven_home(|_| {});
        // Reserved name is refused.
        assert!(matches!(
            create_familiar(writable_familiar("val")),
            Err(FamiliarWriteError::InvalidId(_))
        ));
        // Remove/rename of a missing id errors instead of silently succeeding.
        assert!(matches!(
            remove_familiar("ghost"),
            Err(FamiliarWriteError::NotFound(_))
        ));
        assert!(matches!(
            rename_familiar("ghost", "wraith"),
            Err(FamiliarWriteError::NotFound(_))
        ));
    }

    #[test]
    fn write_refused_when_daemon_socket_present() {
        let _g = with_coven_home(|home| {
            // Simulate a running daemon by creating its IPC socket path.
            std::fs::write(home.join("coven.sock"), b"").unwrap();
        });
        assert!(!can_write_familiars());
        assert!(matches!(
            create_familiar(writable_familiar("nova")),
            Err(FamiliarWriteError::DaemonOwned)
        ));
    }

    #[test]
    fn familiar_access_defaults_to_read_only_when_absent() {
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "researcher"
display_name = "Researcher"
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
id = "builder"
access = "full"

[[familiar]]
id = "researcher"
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
            id: "Builder".to_string(),
            display_name: Some("Builder".to_string()),
            emoji: Some("⚡".to_string()),
            role: Some("Code".to_string()),
            description: Some("Builds and ships.".to_string()),
            pronouns: None,
            access: Some("full".to_string()),
            model: Some("claude-opus-4-8".to_string()),
        };
        let (id, def) = familiar_to_agent_definition(&fam);
        assert_eq!(id, "builder", "id should be lowercased for map keys");
        assert_eq!(def.access, "full");
        // Explicit per-familiar model is threaded into the agent definition.
        assert_eq!(def.model.as_deref(), Some("claude-opus-4-8"));
        assert!(def.visible);
        let prompt = def.prompt.as_deref().unwrap_or("");
        assert!(prompt.contains("Builder"));
        assert!(prompt.contains("Code"));
    }

    #[test]
    fn familiar_to_agent_definition_defaults_to_read_only() {
        let fam = CovenFamiliar {
            id: "researcher".to_string(),
            display_name: None,
            emoji: None,
            role: None,
            description: None,
            pronouns: None,
            access: None,
            model: None,
        };
        let (_id, def) = familiar_to_agent_definition(&fam);
        assert_eq!(def.access, "read-only");
        // No model override → inherits session default.
        assert_eq!(def.model, None);
    }

    #[test]
    fn default_agents_with_familiars_merges_without_clobbering_builtins() {
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "willow"
display_name = "Willow"
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
        // Familiar `willow` was merged in with its declared access.
        assert_eq!(
            merged.get("willow").map(|d| d.access.as_str()),
            Some("full")
        );
    }

    #[test]
    fn default_agents_with_familiars_and_config_keeps_familiar_over_settings_shadow() {
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "willow"
display_name = "Willow"
role = "Code"
"#,
            )
            .unwrap();
        });

        let mut config_agents = std::collections::HashMap::new();
        config_agents.insert(
            "willow".to_string(),
            crate::config::AgentDefinition {
                description: Some("Project-controlled shadow".to_string()),
                model: None,
                temperature: None,
                prompt: Some("Run shell commands".to_string()),
                access: "full".to_string(),
                visible: true,
                max_turns: None,
                color: None,
            },
        );

        let merged = default_agents_with_familiars_and_config(&config_agents);
        let willow = merged.get("willow").expect("familiar should be present");
        assert_eq!(willow.access, DEFAULT_FAMILIAR_ACCESS);
        let prompt = willow.prompt.as_deref().unwrap_or_default();
        assert!(prompt.contains("Willow"));
        assert!(!prompt.contains("Run shell commands"));
    }

    #[test]
    fn default_agents_with_familiars_and_config_protects_builtin_ids() {
        // Security policy: reserved built-in ids (build / plan / explore)
        // carry access tiers that the TUI mode switcher relies on. Project
        // settings must NOT be able to swap a "full"-access `build` agent
        // into the read-only `plan` slot or vice-versa.
        let _g = with_coven_home(|_| {});
        let mut config_agents = std::collections::HashMap::new();
        config_agents.insert(
            "build".to_string(),
            crate::config::AgentDefinition {
                description: Some("Malicious shadow build".to_string()),
                model: None,
                temperature: None,
                prompt: Some("Custom build prompt".to_string()),
                access: "read-only".to_string(),
                visible: true,
                max_turns: None,
                color: None,
            },
        );

        let merged = default_agents_with_familiars_and_config(&config_agents);
        let build = merged.get("build").expect("build agent should be present");
        // The built-in `build` agent has `access = "full"`; project settings
        // cannot override that.
        assert_eq!(build.access, "full");
        assert_ne!(build.description.as_deref(), Some("Malicious shadow build"));
    }

    #[test]
    fn merge_keeps_plan_reserved_when_config_agent_collides() {
        let _g = with_coven_home(|_| {});
        let mut config_agents = std::collections::HashMap::new();
        config_agents.insert(
            "plan".to_string(),
            crate::config::AgentDefinition {
                description: Some("Project-controlled plan".to_string()),
                model: None,
                temperature: None,
                prompt: Some("shadow plan".to_string()),
                access: "full".to_string(),
                visible: true,
                max_turns: None,
                color: None,
            },
        );

        let merged = default_agents_with_familiars_and_config(&config_agents);
        let plan = merged.get("plan").expect("plan agent should be present");
        assert_eq!(plan.access, "read-only");
        let prompt = plan.prompt.as_deref().unwrap_or_default();
        assert!(prompt.contains("You are the plan agent"));
        assert!(!prompt.contains("shadow plan"));
    }

    #[test]
    fn merge_keeps_plan_reserved_when_familiar_collides() {
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "plan"
display_name = "Plan Shadow"
role = "unsafe"
description = "unsafe familiar details"
access = "full"
"#,
            )
            .unwrap();
        });

        let config_agents = std::collections::HashMap::new();
        let merged = default_agents_with_familiars_and_config(&config_agents);
        let plan = merged.get("plan").expect("plan agent should be present");
        assert_eq!(plan.access, "read-only");
        let prompt = plan.prompt.as_deref().unwrap_or_default();
        assert!(prompt.contains("You are the plan agent"));
        assert!(!prompt.contains("Plan Shadow"));
        assert!(!prompt.contains("unsafe"));
        assert_ne!(
            plan.description.as_deref(),
            Some("✨ unsafe — unsafe familiar details")
        );
    }

    #[test]
    fn merge_includes_saved_non_reserved_familiars() {
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "willow"
display_name = "Willow"
role = "Research"
access = "read-only"
"#,
            )
            .unwrap();
        });

        let config_agents = std::collections::HashMap::new();
        let merged = default_agents_with_familiars_and_config(&config_agents);
        let willow = merged
            .get("willow")
            .expect("willow familiar should be present");
        assert_eq!(willow.access, "read-only");
        let prompt = willow.prompt.as_deref().unwrap_or_default();
        assert!(prompt.contains("Research"));
    }

    #[test]
    fn globally_disallowed_names_never_enter_runtime_agent_map() {
        // A familiar claiming a reserved agent name (val) must be filtered out
        // of the merged runtime map, while declared familiar ids are honored
        // dynamically instead of blocked by a built-in roster.
        let _g = with_coven_home(|home| {
            fs::write(
                home.join("familiars.toml"),
                r#"
[[familiar]]
id = "val"
display_name = "Val"
access = "full"

[[familiar]]
id = "researcher"
display_name = "Researcher"
access = "full"

[[familiar]]
id = "builder"
display_name = "Builder"
access = "full"

[[familiar]]
id = "willow"
display_name = "Willow"
access = "read-only"
"#,
            )
            .unwrap();
        });

        // A project setting also cannot smuggle in the reserved agent name.
        let mut config_agents = std::collections::HashMap::new();
        config_agents.insert(
            "valentina".to_string(),
            crate::config::AgentDefinition {
                description: Some("shadow".to_string()),
                model: None,
                temperature: None,
                prompt: Some("shadow".to_string()),
                access: "full".to_string(),
                visible: true,
                max_turns: None,
                color: None,
            },
        );

        let merged = default_agents_with_familiars_and_config(&config_agents);
        assert!(!merged.contains_key("val"), "val (reserved agent) filtered");
        assert!(
            !merged.contains_key("valentina"),
            "valentina (reserved agent, via settings) filtered"
        );
        assert!(
            merged.contains_key("researcher"),
            "declared familiar id survives without a built-in roster"
        );
        assert!(
            merged.contains_key("builder"),
            "declared familiar id survives without a built-in roster"
        );
        assert!(merged.contains_key("willow"), "free name survives");
    }

    #[test]
    fn disallowed_name_helpers_are_case_insensitive() {
        assert!(is_disallowed_agent_name("Val"));
        assert!(is_disallowed_agent_name("  VALENTINA "));
        assert!(!is_disallowed_agent_name("researcher"));
        assert!(!is_disallowed_familiar_name("Researcher"));
        assert!(!is_disallowed_familiar_name("BUILDER"));
        assert!(is_disallowed_familiar_name("val")); // agent ban ⊆ familiar ban
        assert!(!is_disallowed_familiar_name("willow"));
    }

    #[test]
    fn canonicalize_access_tier_accepts_canonical_lowercase() {
        assert_eq!(canonicalize_access_tier("full"), Some("full"));
        assert_eq!(canonicalize_access_tier("read-only"), Some("read-only"));
        assert_eq!(canonicalize_access_tier("search-only"), Some("search-only"));
    }

    #[test]
    fn canonicalize_access_tier_normalizes_case_and_whitespace() {
        assert_eq!(canonicalize_access_tier("FULL"), Some("full"));
        assert_eq!(canonicalize_access_tier("Read-Only"), Some("read-only"));
        assert_eq!(
            canonicalize_access_tier("  search-only\n"),
            Some("search-only")
        );
        assert_eq!(canonicalize_access_tier(" full "), Some("full"));
    }

    #[test]
    fn canonicalize_access_tier_rejects_unknown_strings() {
        // Typos and near-matches must NOT round-trip — callers depend on
        // `None` to trigger fail-closed behavior.
        for unknown in &[
            "readonly",
            "Full Access",
            "writable",
            "",
            "rad-only",
            "search only",
        ] {
            assert!(
                canonicalize_access_tier(unknown).is_none(),
                "expected {unknown:?} to be rejected"
            );
        }
    }

    #[test]
    fn resolve_access_tier_falls_back_to_default_on_unknown() {
        assert_eq!(resolve_access_tier("readonly"), DEFAULT_FAMILIAR_ACCESS);
        assert_eq!(resolve_access_tier(""), DEFAULT_FAMILIAR_ACCESS);
        assert_eq!(resolve_access_tier("i-am-evil"), DEFAULT_FAMILIAR_ACCESS);
    }

    #[test]
    fn resolve_access_tier_passes_canonical_through() {
        assert_eq!(resolve_access_tier("full"), "full");
        assert_eq!(resolve_access_tier("read-only"), "read-only");
        assert_eq!(resolve_access_tier("search-only"), "search-only");
        // Case + whitespace are part of the "canonicalize silently" contract.
        assert_eq!(resolve_access_tier(" FULL "), "full");
        assert_eq!(resolve_access_tier("READ-ONLY"), "read-only");
    }

    #[test]
    fn familiar_resolved_access_normalizes_case_variants() {
        // Typos and case mismatches in `~/.coven/familiars.toml` must NOT
        // grant a familiar more power than the user intended. Case variants
        // canonicalize silently; truly unknown values fail closed to the
        // restrictive default.
        let case_variant = CovenFamiliar {
            id: "rogue".into(),
            display_name: None,
            emoji: None,
            role: None,
            description: None,
            pronouns: None,
            access: Some("READ-ONLY".into()),
            model: None,
        };
        assert_eq!(case_variant.resolved_access(), "read-only");

        let typo = CovenFamiliar {
            id: "rogue".into(),
            display_name: None,
            emoji: None,
            role: None,
            description: None,
            pronouns: None,
            access: Some("readonly".into()),
            model: None,
        };
        assert_eq!(typo.resolved_access(), DEFAULT_FAMILIAR_ACCESS);

        let garbage = CovenFamiliar {
            id: "rogue".into(),
            display_name: None,
            emoji: None,
            role: None,
            description: None,
            pronouns: None,
            access: Some("super-admin".into()),
            model: None,
        };
        assert_eq!(garbage.resolved_access(), DEFAULT_FAMILIAR_ACCESS);
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
