//! Skill discovery: load custom prompt-template skills from markdown files
//! on disk and (optionally) from git URLs.
//!
//! Skills are inherited from several environments so coven-code surfaces the
//! same skill set as the surrounding tooling. Search priority (first match
//! wins for a given skill name):
//!   1. Project (walk up from `cwd`):
//!      `.coven-code/skills/`, `.agents/skills/`, `.claude/skills/`
//!   2. User (home dir):
//!      `~/.coven-code/skills/`, `~/.claude/skills/`, `~/.codex/prompts/`
//!   3. Plugin: `~/.claude/plugins/*/skills/`
//!   4. Configured extra paths from `SkillsConfig.paths`
//!   5. Git-URL repos from `SkillsConfig.urls` (cloned once, then cached)
//!
//! Two on-disk layouts are supported per root:
//!   - **Directory layout** (`<name>/SKILL.md`) — used by Claude / superpowers
//!     plugins. Frontmatter `name` / `description` / `when-to-use`.
//!   - **Flat layout** (`<name>.md`) — used by coven skills and Codex prompts
//!     (`~/.codex/prompts/`). Codex prompts have no frontmatter, so the file
//!     stem is the name and the first body line is the description.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Where a discovered skill came from. Drives the scope label shown in the
/// `/skills` picker (`builtin` is reserved for in-binary bundled skills, which
/// are not produced by this module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillScope {
    /// Found under a project directory (walked up from cwd).
    Project,
    /// Found under the user's home directory.
    User,
    /// Contributed by an installed plugin.
    Plugin,
}

impl SkillScope {
    /// Short word rendered in the picker (`project` / `user` / `plugin`).
    pub fn label(self) -> &'static str {
        match self {
            SkillScope::Project => "project",
            SkillScope::User => "user",
            SkillScope::Plugin => "plugin",
        }
    }
}

/// Rough token estimate for a string, for display only (not an exact count).
///
/// Delegates to the canonical [`crate::message_utils::estimate_tokens`] so the
/// token heuristic stays single-sourced and user-facing counts don't drift.
pub fn estimate_tokens(text: &str) -> usize {
    crate::message_utils::estimate_tokens(text) as usize
}

/// A discovered skill loaded from a markdown file.
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    /// Skill name (from `name:` frontmatter or file stem / dir name).
    pub name: String,
    /// One-line description (from `description:` frontmatter or first body line).
    pub description: String,
    /// Optional "when to use" guidance from frontmatter.
    pub when_to_use: Option<String>,
    /// The prompt body after stripping frontmatter.
    pub template: String,
    /// Absolute path to the source `.md` file.
    pub source_path: PathBuf,
    /// Which environment this skill was inherited from.
    pub scope: SkillScope,
    /// Human label of the origin (`coven`, `claude`, `codex`, or plugin name).
    pub origin: String,
    /// Estimated always-on context cost (name + description + when-to-use).
    pub est_tokens: usize,
}

impl DiscoveredSkill {
    /// Recompute `est_tokens` from the current metadata fields.
    fn recompute_tokens(&mut self) {
        let meta = format!(
            "{} {} {}",
            self.name,
            self.description,
            self.when_to_use.as_deref().unwrap_or("")
        );
        self.est_tokens = estimate_tokens(&meta);
    }
}

// ---------------------------------------------------------------------------
// Frontmatter parsing
// ---------------------------------------------------------------------------

/// Parse a skill markdown file.
///
/// Expects optional YAML frontmatter delimited by `---`. When `description`
/// is absent (e.g. Codex prompts), it falls back to the first non-empty body
/// line (with any leading `#` heading markers stripped). Returns `None` when
/// the file is empty after trimming.
///
/// `scope` / `origin` are stamped to defaults here; the scanning layer
/// (`scan_skill_root`) overrides them for each root.
pub fn parse_skill_file(content: &str, path: &Path) -> Option<DiscoveredSkill> {
    let content = content.trim();
    if content.is_empty() {
        return None;
    }

    let (name, description, when_to_use, template) =
        if let Some(after_open) = content.strip_prefix("---") {
            // Accept both `\n---` and `\r\n---` as closing delimiter.
            if let Some(close_pos) = after_open.find("\n---") {
                let frontmatter = &after_open[..close_pos];
                let rest = after_open[close_pos + 4..].trim_start_matches(['\r', '\n']);
                let (name, description, when_to_use) = parse_frontmatter(frontmatter);
                (name, description, when_to_use, rest.to_string())
            } else {
                // Malformed frontmatter — treat entire content as template.
                (None, None, None, content.to_string())
            }
        } else {
            (None, None, None, content.to_string())
        };

    // Treat a present-but-empty `name:` as missing so it can't become an
    // empty HashMap key that breaks dedupe/lookup; fall back to the file stem.
    let name = name.filter(|n| !n.is_empty()).unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed")
            .to_string()
    });
    // Fall back to the first meaningful body line so frontmatter-less skills
    // (Codex prompts, bare `.md` files) still get a useful one-liner.
    let description = description
        .filter(|d| !d.is_empty())
        .or_else(|| first_body_line(&template))
        .unwrap_or_else(|| "Custom skill".to_string());

    if template.is_empty() && name.is_empty() {
        return None;
    }

    let mut skill = DiscoveredSkill {
        name,
        description,
        when_to_use,
        template,
        source_path: path.to_path_buf(),
        scope: SkillScope::User,
        origin: String::new(),
        est_tokens: 0,
    };
    skill.recompute_tokens();
    Some(skill)
}

/// Strip surrounding single/double quotes and whitespace from a frontmatter
/// value.
fn unquote(v: &str) -> String {
    v.trim().trim_matches('"').trim_matches('\'').to_string()
}

/// Parse the YAML frontmatter for `name`, `description`, and `when-to-use`,
/// handling both inline values (`description: text`) and block scalars
/// (`description: |` / `>` followed by indented continuation lines, as used by
/// the Claude/superpowers skill set). Block scalars are folded to a single
/// space-joined line — enough for display and token estimation.
fn parse_frontmatter(frontmatter: &str) -> (Option<String>, Option<String>, Option<String>) {
    let lines: Vec<&str> = frontmatter.lines().collect();
    let mut name = None;
    let mut description = None;
    let mut when_to_use = None;

    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let indent = raw.len() - raw.trim_start().len();
        i += 1;

        let Some((key, val)) = raw.trim().split_once(':') else {
            continue;
        };
        let key = key.trim();
        if !matches!(key, "name" | "description" | "when-to-use" | "when_to_use") {
            continue;
        }
        let val = val.trim();

        // Block scalar: empty value or a `|` / `>` indicator → gather the
        // following lines that are indented deeper than this key.
        let value = if val.is_empty() || val.starts_with('|') || val.starts_with('>') {
            let mut parts: Vec<String> = Vec::new();
            while i < lines.len() {
                let cont = lines[i];
                if cont.trim().is_empty() {
                    i += 1;
                    continue;
                }
                let cont_indent = cont.len() - cont.trim_start().len();
                if cont_indent > indent {
                    parts.push(cont.trim().to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            parts.join(" ")
        } else {
            unquote(val)
        };

        match key {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => when_to_use = Some(value),
        }
    }

    (name, description, when_to_use)
}

/// First non-empty line of a body, with any leading `#` heading markers
/// stripped, truncated to 200 chars.
fn first_body_line(body: &str) -> Option<String> {
    for line in body.lines() {
        let t = line.trim().trim_start_matches('#').trim();
        if !t.is_empty() {
            let truncated: String = t.chars().take(200).collect();
            return Some(truncated);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Directory scanning
// ---------------------------------------------------------------------------

/// Scan a single skill root, handling both layouts:
///   - flat `<name>.md` files directly in `dir`
///   - `<name>/SKILL.md` directories (Claude / superpowers / plugins)
///
/// Each returned skill is stamped with `scope` and `origin`.
fn scan_skill_root(dir: &Path, scope: SkillScope, origin: &str) -> Vec<DiscoveredSkill> {
    let mut skills = Vec::new();
    if !dir.is_dir() {
        return skills;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            tracing::debug!(dir = %dir.display(), error = %err, "skill_discovery: read_dir failed");
            return skills;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let parsed = if path.is_dir() {
            // Directory layout: look for `SKILL.md` (or lowercase `skill.md`)
            // and default the name to the directory name.
            let skill_md = path.join("SKILL.md");
            let skill_md = if skill_md.is_file() {
                Some(skill_md)
            } else {
                let alt = path.join("skill.md");
                alt.is_file().then_some(alt)
            };
            skill_md.and_then(|p| read_and_parse(&p)).map(|mut s| {
                // Prefer the directory name when frontmatter omitted `name:`.
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    if s.name == "SKILL" || s.name == "skill" {
                        s.name = dir_name.to_string();
                        s.recompute_tokens();
                    }
                }
                s
            })
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            read_and_parse(&path)
        } else {
            None
        };

        if let Some(mut skill) = parsed {
            skill.scope = scope;
            skill.origin = origin.to_string();
            skills.push(skill);
        }
    }

    skills
}

/// Read a file and parse it into a skill, logging read failures.
fn read_and_parse(path: &Path) -> Option<DiscoveredSkill> {
    match std::fs::read_to_string(path) {
        Ok(content) => parse_skill_file(&content, path),
        Err(err) => {
            tracing::debug!(path = %path.display(), error = %err, "skill_discovery: read failed");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level discovery
// ---------------------------------------------------------------------------

/// Discover all skills from all configured sources.
///
/// Returns a `HashMap` of `skill_name → DiscoveredSkill` (first match wins;
/// duplicates from lower-priority sources are logged at debug level).
pub fn discover_skills(
    cwd: &Path,
    config_skills: &crate::config::SkillsConfig,
) -> HashMap<String, DiscoveredSkill> {
    let mut all: HashMap<String, DiscoveredSkill> = HashMap::new();
    let mut warn_duplicates: Vec<String> = Vec::new();

    // Inline closure: insert a batch, warning on duplicates (first wins).
    let mut add = |skills: Vec<DiscoveredSkill>| {
        for skill in skills {
            if let Some(existing) = all.get(&skill.name) {
                warn_duplicates.push(format!(
                    "Duplicate skill '{}' found at {} (keeping {})",
                    skill.name,
                    skill.source_path.display(),
                    existing.source_path.display()
                ));
            } else {
                all.insert(skill.name.clone(), skill);
            }
        }
    };

    let home = dirs::home_dir();

    // ---- 1. Project skills: walk up from cwd --------------------------------
    // Stop at the home directory so home-level skill dirs are attributed to the
    // User scope below (not "project") when the checkout lives under $HOME.
    {
        let mut dir: &Path = cwd;
        loop {
            if home.as_deref() == Some(dir) {
                break;
            }
            add(scan_skill_root(
                &dir.join(".coven-code").join("skills"),
                SkillScope::Project,
                "coven",
            ));
            add(scan_skill_root(
                &dir.join(".agents").join("skills"),
                SkillScope::Project,
                "agents",
            ));
            add(scan_skill_root(
                &dir.join(".claude").join("skills"),
                SkillScope::Project,
                "claude",
            ));
            match dir.parent() {
                Some(parent) if parent != dir => dir = parent,
                _ => break,
            }
        }
    }

    // ---- 2. User skills: home directory -------------------------------------
    if let Some(home) = home {
        add(scan_skill_root(
            &home.join(".coven-code").join("skills"),
            SkillScope::User,
            "coven",
        ));
        add(scan_skill_root(
            &home.join(".agents").join("skills"),
            SkillScope::User,
            "agents",
        ));
        add(scan_skill_root(
            &home.join(".claude").join("skills"),
            SkillScope::User,
            "claude",
        ));
        // OpenAI Codex custom prompts (flat `.md`, no frontmatter).
        add(scan_skill_root(
            &home.join(".codex").join("prompts"),
            SkillScope::User,
            "codex",
        ));

        // ---- 3. Plugin skills: ~/.claude/plugins/*/skills/ -----------------
        let plugins_root = home.join(".claude").join("plugins");
        if let Ok(entries) = std::fs::read_dir(&plugins_root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let origin = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("plugin")
                        .to_string();
                    add(scan_skill_root(
                        &p.join("skills"),
                        SkillScope::Plugin,
                        &origin,
                    ));
                }
            }
        }
    }

    // ---- 4. Configured extra paths ------------------------------------------
    for path_str in &config_skills.paths {
        let path = Path::new(path_str);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        add(scan_skill_root(&path, SkillScope::User, "config"));
    }

    // ---- 5. Git URL skills (cached) -----------------------------------------
    for url in &config_skills.urls {
        if let Some(git_skills) = fetch_git_skills(url) {
            add(git_skills);
        }
    }

    // Duplicates are expected now that skills are inherited from several
    // overlapping environments (e.g. the same skill in ~/.claude and
    // ~/.agents), so log them at debug level rather than warn.
    for w in &warn_duplicates {
        tracing::debug!("{}", w);
    }

    all
}

// ---------------------------------------------------------------------------
// Git URL support
// ---------------------------------------------------------------------------

/// Clone or reuse a cached git repo and return skills found in it.
///
/// Cache location: `<system-cache>/coven-code/skills/<repo-name>/`
/// On first access the repo is cloned with `--depth=1`.
/// Subsequent calls use the already-cloned cache directory as-is.
fn fetch_git_skills(url: &str) -> Option<Vec<DiscoveredSkill>> {
    let cache_dir = dirs::cache_dir()?.join("coven-code").join("skills");

    // Use the last path segment of the URL as the local directory name.
    let repo_name = url.split('/').next_back()?.trim_end_matches(".git");

    if repo_name.is_empty() {
        tracing::warn!(url, "skill_discovery: cannot derive repo name from git URL");
        return None;
    }

    let repo_dir = cache_dir.join(repo_name);

    if !repo_dir.exists() {
        tracing::info!(url, dest = %repo_dir.display(), "skill_discovery: cloning skills repo");

        // Ensure the parent cache directory exists.
        if let Err(err) = std::fs::create_dir_all(&cache_dir) {
            tracing::warn!(
                dir = %cache_dir.display(),
                error = %err,
                "skill_discovery: could not create cache dir"
            );
            return None;
        }

        let repo_dir_str = repo_dir.to_str()?;
        let status = std::process::Command::new("git")
            .args(["clone", "--depth=1", url, repo_dir_str])
            .status();

        match status {
            Ok(s) if s.success() => {
                tracing::info!(url, "skill_discovery: clone succeeded");
            }
            Ok(s) => {
                tracing::warn!(url, exit_code = ?s.code(), "skill_discovery: git clone failed");
                return None;
            }
            Err(err) => {
                tracing::warn!(url, error = %err, "skill_discovery: could not spawn git");
                return None;
            }
        }
    }

    // Scan repo root and optional `skills/` subdirectory.
    let mut skills = scan_skill_root(&repo_dir, SkillScope::User, repo_name);
    skills.extend(scan_skill_root(
        &repo_dir.join("skills"),
        SkillScope::User,
        repo_name,
    ));
    Some(skills)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;

    fn make_temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn write_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    // ---- parse_skill_file ---------------------------------------------------

    #[test]
    fn test_parse_with_frontmatter() {
        let content =
            "---\nname: review\ndescription: Review code changes\n---\n\nPlease review $ARGUMENTS";
        let path = PathBuf::from("review.md");
        let skill = parse_skill_file(content, &path).unwrap();
        assert_eq!(skill.name, "review");
        assert_eq!(skill.description, "Review code changes");
        assert!(skill.template.contains("$ARGUMENTS"));
    }

    #[test]
    fn test_parse_no_frontmatter_uses_stem_and_first_line() {
        // Codex-style flat prompt: no frontmatter, description from first line.
        let content = "Do something useful.";
        let path = PathBuf::from("my-skill.md");
        let skill = parse_skill_file(content, &path).unwrap();
        assert_eq!(skill.name, "my-skill");
        assert_eq!(skill.description, "Do something useful.");
        assert_eq!(skill.template, "Do something useful.");
        assert!(skill.est_tokens > 0);
    }

    #[test]
    fn test_parse_when_to_use_frontmatter() {
        let content =
            "---\nname: brainstorm\ndescription: Explore ideas\nwhen-to-use: before any creative work\n---\nBody.";
        let skill = parse_skill_file(content, &PathBuf::from("x.md")).unwrap();
        assert_eq!(
            skill.when_to_use.as_deref(),
            Some("before any creative work")
        );
    }

    #[test]
    fn test_parse_block_scalar_description() {
        // Mirrors the Claude/superpowers `description: |` block-scalar layout.
        let content = "---\nversion: 0.3.0\nname: higgs\ndescription: |\n  First line of the description.\n  Second line continues here.\n---\n# Body\nstuff";
        let skill = parse_skill_file(content, &PathBuf::from("higgs/SKILL.md")).unwrap();
        assert_eq!(skill.name, "higgs");
        assert_eq!(
            skill.description,
            "First line of the description. Second line continues here."
        );
        // Token estimate should reflect the full folded description, not just "|".
        assert!(skill.est_tokens >= 12, "got {}", skill.est_tokens);
    }

    #[test]
    fn test_estimate_tokens_monotonic() {
        assert!(estimate_tokens("a much longer string here") > estimate_tokens("short"));
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_scan_skill_root_directory_layout() {
        let tmp = make_temp_dir();
        // <root>/brainstorming/SKILL.md  (name omitted → dir name used)
        write_file(
            tmp.path(),
            "brainstorming/SKILL.md",
            "---\ndescription: Turn ideas into designs\n---\nDo the thing.",
        );
        let skills = scan_skill_root(tmp.path(), SkillScope::Plugin, "superpowers");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "brainstorming");
        assert_eq!(skills[0].scope, SkillScope::Plugin);
        assert_eq!(skills[0].origin, "superpowers");
        assert_eq!(skills[0].description, "Turn ideas into designs");
    }

    #[test]
    fn test_discover_stamps_project_scope() {
        let tmp = make_temp_dir();
        let skills_dir = tmp.path().join(".coven-code").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        write_file(&skills_dir, "myskill.md", "---\nname: myskill\n---\nDo it.");

        let config = crate::config::SkillsConfig::default();
        let discovered = discover_skills(tmp.path(), &config);
        assert_eq!(discovered["myskill"].scope, SkillScope::Project);
    }

    #[test]
    fn test_parse_missing_name_uses_stem() {
        let content = "---\ndescription: No name field\n---\n\nBody text.";
        let path = PathBuf::from("fallback.md");
        let skill = parse_skill_file(content, &path).unwrap();
        assert_eq!(skill.name, "fallback");
        assert_eq!(skill.description, "No name field");
    }

    #[test]
    fn test_parse_empty_returns_none() {
        let skill = parse_skill_file("   ", &PathBuf::from("empty.md"));
        assert!(skill.is_none());
    }

    #[test]
    fn test_parse_quoted_frontmatter_values() {
        let content = "---\nname: \"quoted name\"\ndescription: 'single quoted'\n---\nBody.";
        let skill = parse_skill_file(content, &PathBuf::from("x.md")).unwrap();
        assert_eq!(skill.name, "quoted name");
        assert_eq!(skill.description, "single quoted");
    }

    // ---- scan_dir -----------------------------------------------------------

    #[test]
    fn test_scan_dir_finds_skills() {
        let tmp = make_temp_dir();
        write_file(
            tmp.path(),
            "review.md",
            "---\nname: review\n---\nReview $ARGUMENTS",
        );
        write_file(tmp.path(), "debug.md", "Debug help.");
        write_file(tmp.path(), "not-md.txt", "ignored");

        let skills = scan_skill_root(tmp.path(), SkillScope::User, "");
        assert_eq!(skills.len(), 2);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"review"));
        assert!(names.contains(&"debug"));
    }

    #[test]
    fn test_scan_dir_nonexistent_returns_empty() {
        let skills = scan_skill_root(Path::new("/nonexistent/path/xyz"), SkillScope::User, "");
        assert!(skills.is_empty());
    }

    // ---- discover_skills ----------------------------------------------------

    #[test]
    fn test_discover_from_project_dir() {
        let tmp = make_temp_dir();
        let skills_dir = tmp.path().join(".coven-code").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        write_file(
            &skills_dir,
            "myskill.md",
            "---\nname: myskill\ndescription: Test\n---\nDo it.",
        );

        let config = crate::config::SkillsConfig::default();
        let discovered = discover_skills(tmp.path(), &config);
        assert!(discovered.contains_key("myskill"));
        assert_eq!(discovered["myskill"].description, "Test");
    }

    #[test]
    fn test_discover_extra_paths() {
        let tmp = make_temp_dir();
        let extra = make_temp_dir();
        write_file(
            extra.path(),
            "extra.md",
            "---\nname: extra\n---\nExtra skill.",
        );

        let config = crate::config::SkillsConfig {
            paths: vec![extra.path().to_str().unwrap().to_string()],
            urls: vec![],
        };
        let discovered = discover_skills(tmp.path(), &config);
        assert!(discovered.contains_key("extra"));
    }

    #[test]
    fn test_discover_deduplicates_first_wins() {
        let tmp = make_temp_dir();
        let proj_skills = tmp.path().join(".coven-code").join("skills");
        std::fs::create_dir_all(&proj_skills).unwrap();
        write_file(
            &proj_skills,
            "dup.md",
            "---\nname: dup\ndescription: project\n---\nProject.",
        );

        let extra = make_temp_dir();
        write_file(
            extra.path(),
            "dup.md",
            "---\nname: dup\ndescription: extra\n---\nExtra.",
        );

        let config = crate::config::SkillsConfig {
            paths: vec![extra.path().to_str().unwrap().to_string()],
            urls: vec![],
        };
        let discovered = discover_skills(tmp.path(), &config);
        // Project-level wins over extra path.
        assert_eq!(discovered["dup"].description, "project");
    }
}
