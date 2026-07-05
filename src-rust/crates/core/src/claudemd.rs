//! AGENTS.md hierarchical memory loading.
//! Mirrors src/utils/claudemd.ts (1,479 lines).
//!
//! Priority order: managed > user > project > local
//! Supports @include directives, YAML frontmatter, and mtime-based caching.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::hosted_review::RuntimeMode;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Memory file type / priority scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// `~/.coven-code/rules/*.md` — global managed policy.
    Managed,
    /// `~/.coven-code/AGENTS.md` — user-level memory.
    User,
    /// `{project_root}/AGENTS.md` — project-level memory.
    Project,
    /// `{project_root}/.coven-code/AGENTS.md` — local override.
    Local,
}

/// Frontmatter parsed from a AGENTS.md file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryFrontmatter {
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Loaded memory file with metadata.
#[derive(Debug, Clone)]
pub struct MemoryFileInfo {
    pub path: PathBuf,
    pub scope: MemoryScope,
    pub content: String,
    pub frontmatter: MemoryFrontmatter,
    pub mtime: Option<SystemTime>,
}

/// Controls which memory scopes are loaded for the current runtime mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryLoadOptions {
    pub mode: RuntimeMode,
    pub allow_user_memory: bool,
    pub allow_managed_rules: bool,
}

impl MemoryLoadOptions {
    pub fn local() -> Self {
        Self {
            mode: RuntimeMode::Local,
            allow_user_memory: true,
            allow_managed_rules: true,
        }
    }

    pub fn hosted_review() -> Self {
        Self {
            mode: RuntimeMode::HostedReview,
            allow_user_memory: false,
            allow_managed_rules: false,
        }
    }

    pub fn from_mode(mode: RuntimeMode) -> Self {
        match mode {
            RuntimeMode::Local => Self::local(),
            RuntimeMode::HostedReview => Self::hosted_review(),
        }
    }
}

fn memory_home_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Ok(path) = std::env::var("COVEN_CODE_TEST_HOME") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    dirs::home_dir()
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// Simple mtime-keyed file cache.
#[derive(Default)]
pub struct MemoryCache {
    entries: HashMap<PathBuf, (SystemTime, String)>,
}

impl MemoryCache {
    /// Return cached content if the file hasn't changed since last read.
    pub fn get(&self, path: &Path) -> Option<&str> {
        let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
        let (cached_mtime, content) = self.entries.get(path)?;
        if *cached_mtime == mtime {
            Some(content.as_str())
        } else {
            None
        }
    }

    /// Store file content with its current mtime.
    pub fn insert(&mut self, path: PathBuf, content: String) {
        if let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) {
            self.entries.insert(path, (mtime, content));
        }
    }
}

// ---------------------------------------------------------------------------
// YAML frontmatter parsing
// ---------------------------------------------------------------------------

/// Strip YAML frontmatter (--- ... ---) from content and parse it.
/// Returns (frontmatter, body_without_frontmatter).
pub fn parse_frontmatter(content: &str) -> (MemoryFrontmatter, &str) {
    if !content.starts_with("---") {
        return (MemoryFrontmatter::default(), content);
    }
    let after_first = &content[3..];
    if let Some(end) = after_first.find("\n---") {
        let yaml = after_first[..end].trim();
        let body = &after_first[end + 4..];
        // Minimal YAML key-value parse (no external dependency).
        let mut fm = MemoryFrontmatter::default();
        for line in yaml.lines() {
            let line = line.trim();
            if let Some((key, val)) = line.split_once(':') {
                let val = val.trim().to_string();
                match key.trim() {
                    "memory_type" => fm.memory_type = Some(val),
                    "priority" => fm.priority = val.parse().ok(),
                    "scope" => fm.scope = Some(val),
                    _ => {}
                }
            }
        }
        return (fm, body.trim_start_matches('\n'));
    }
    (MemoryFrontmatter::default(), content)
}

// ---------------------------------------------------------------------------
// @include directive expansion
// ---------------------------------------------------------------------------

/// Maximum @include nesting depth.
const MAX_INCLUDE_DEPTH: usize = 10;

/// Expand @include directives in content.
/// Circular references are detected via `visited` set.
pub fn expand_includes(
    content: &str,
    base_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> String {
    if depth >= MAX_INCLUDE_DEPTH {
        return content.to_string();
    }

    let mut result = String::with_capacity(content.len());
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(path_str) = trimmed.strip_prefix("@include ") {
            let path_str = path_str.trim();
            // Resolve relative to base_dir; expand ~ to home dir.
            let include_path = if path_str.starts_with('~') {
                memory_home_dir().unwrap_or_default().join(&path_str[2..])
            } else if Path::new(path_str).is_absolute() {
                PathBuf::from(path_str)
            } else {
                base_dir.join(path_str)
            };

            let canonical = include_path.canonicalize().unwrap_or(include_path.clone());
            if visited.contains(&canonical) {
                result.push_str(&format!(
                    "<!-- circular @include {} skipped -->\n",
                    path_str
                ));
                continue;
            }
            if let Ok(included) = std::fs::read_to_string(&include_path) {
                // Check max size.
                if included.len() > 40 * 1024 {
                    result.push_str(&format!(
                        "<!-- @include {} exceeds 40KB limit -->\n",
                        path_str
                    ));
                    continue;
                }
                visited.insert(canonical);
                let expanded = expand_includes(
                    &included,
                    include_path.parent().unwrap_or(base_dir),
                    visited,
                    depth + 1,
                );
                result.push_str(&expanded);
                result.push('\n');
            } else {
                result.push_str(&format!("<!-- @include {} not found -->\n", path_str));
            }
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Loading API
// ---------------------------------------------------------------------------

const MAX_FILE_SIZE: u64 = 40 * 1024; // 40 KB

/// Load a single AGENTS.md file (respects MAX_FILE_SIZE, expands @includes).
pub fn load_memory_file(path: &Path, scope: MemoryScope) -> Option<MemoryFileInfo> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_FILE_SIZE {
        eprintln!("WARNING: {} exceeds 40KB limit, skipping", path.display());
        return None;
    }
    let raw = std::fs::read_to_string(path).ok()?;
    let mtime = meta.modified().ok();

    let (frontmatter, body) = parse_frontmatter(&raw);
    let mut visited = HashSet::new();
    visited.insert(path.canonicalize().unwrap_or(path.to_path_buf()));
    let content = expand_includes(
        body,
        path.parent().unwrap_or(Path::new(".")),
        &mut visited,
        0,
    );

    Some(MemoryFileInfo {
        path: path.to_path_buf(),
        scope,
        content,
        frontmatter,
        mtime,
    })
}

/// Load memory files from a directory for a given scope.
///
/// Loads `AGENTS.md` first (primary/universal standard), then `CLAUDE.md` if
/// present (Claude-specific additions or overrides). Either file may be absent.
fn load_scope_files(dir: &Path, scope: MemoryScope, files: &mut Vec<MemoryFileInfo>) {
    for name in &["AGENTS.md", "CLAUDE.md"] {
        let path = dir.join(name);
        if path.exists() {
            if let Some(f) = load_memory_file(&path, scope) {
                files.push(f);
            }
        }
    }
}

/// Load all memory files for the given project root, in priority order.
///
/// At each scope `AGENTS.md` is loaded first (universal standard), followed by
/// `CLAUDE.md` if present (Claude-specific context). Either or both may exist.
///
/// Returned list is ordered: Managed (highest) → User → Project → Local.
pub fn load_all_memory_files(project_root: &Path) -> Vec<MemoryFileInfo> {
    load_all_memory_files_with_options(project_root, &MemoryLoadOptions::local())
}

/// Load all memory files for the given project root using explicit scope gates.
pub fn load_all_memory_files_with_options(
    project_root: &Path,
    options: &MemoryLoadOptions,
) -> Vec<MemoryFileInfo> {
    let mut files = Vec::new();

    // 1. Managed: ~/.coven-code/rules/*.md
    if let Some(home) = memory_home_dir() {
        if options.allow_managed_rules {
            let rules_dir = home.join(".coven-code/rules");
            if let Ok(entries) = std::fs::read_dir(&rules_dir) {
                let mut paths: Vec<PathBuf> = entries
                    .flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        if p.extension().is_some_and(|x| x == "md") {
                            Some(p)
                        } else {
                            None
                        }
                    })
                    .collect();
                paths.sort();
                for p in paths {
                    if let Some(f) = load_memory_file(&p, MemoryScope::Managed) {
                        files.push(f);
                    }
                }
            }
        }

        // 2. User: ~/.coven-code/AGENTS.md then ~/.coven-code/CLAUDE.md
        if options.allow_user_memory {
            load_scope_files(&home.join(".coven-code"), MemoryScope::User, &mut files);
        }
    }

    // 3. Project: {project_root}/AGENTS.md then {project_root}/CLAUDE.md
    load_scope_files(project_root, MemoryScope::Project, &mut files);

    // 4. Local: {project_root}/.coven-code/AGENTS.md then {project_root}/.coven-code/CLAUDE.md
    load_scope_files(
        &project_root.join(".coven-code"),
        MemoryScope::Local,
        &mut files,
    );

    files
}

/// Concatenate all memory file contents into a single system-prompt fragment.
pub fn build_memory_prompt(files: &[MemoryFileInfo]) -> String {
    files
        .iter()
        .filter(|f| !f.content.trim().is_empty())
        .map(|f| f.content.trim().to_string())
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_basic() {
        let content = "---\nmemory_type: project\npriority: 10\n---\nHello world";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.memory_type.as_deref(), Some("project"));
        assert_eq!(fm.priority, Some(10));
        assert_eq!(body.trim(), "Hello world");
    }

    #[test]
    fn parse_frontmatter_none() {
        let content = "No frontmatter here";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.memory_type.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn load_scope_prefers_agents_then_claude() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();
        std::fs::write(tmp.path().join("CLAUDE.md"), "claude content").unwrap();

        let files = load_all_memory_files(tmp.path());
        // Filter to just the project-scope files from our temp dir.
        let project: Vec<_> = files
            .iter()
            .filter(|f| f.path.starts_with(tmp.path()))
            .collect();
        assert_eq!(
            project.len(),
            2,
            "both AGENTS.md and CLAUDE.md should be loaded"
        );
        assert!(
            project[0].path.ends_with("AGENTS.md"),
            "AGENTS.md must come first"
        );
        assert!(
            project[1].path.ends_with("CLAUDE.md"),
            "CLAUDE.md must follow"
        );
    }

    #[test]
    fn load_scope_claudemd_only_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("CLAUDE.md"), "claude only").unwrap();

        let files = load_all_memory_files(tmp.path());
        let project: Vec<_> = files
            .iter()
            .filter(|f| f.path.starts_with(tmp.path()))
            .collect();
        assert_eq!(project.len(), 1);
        assert!(project[0].path.ends_with("CLAUDE.md"));
    }

    #[test]
    fn hosted_review_excludes_user_memory_by_default() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("AGENTS.md"), "project memory").unwrap();

        let home = tempfile::tempdir().unwrap();
        let coven_code = home.path().join(".coven-code");
        std::fs::create_dir_all(&coven_code).unwrap();
        std::fs::write(coven_code.join("AGENTS.md"), "user memory").unwrap();

        let _lock = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let original_test_home = std::env::var("COVEN_CODE_TEST_HOME").ok();
        let original_home = std::env::var("HOME").ok();
        let original_userprofile = std::env::var("USERPROFILE").ok();
        std::env::set_var("COVEN_CODE_TEST_HOME", home.path());
        std::env::set_var("HOME", home.path());
        std::env::set_var("USERPROFILE", home.path());

        let files =
            load_all_memory_files_with_options(project.path(), &MemoryLoadOptions::hosted_review());

        match original_test_home {
            Some(value) => std::env::set_var("COVEN_CODE_TEST_HOME", value),
            None => std::env::remove_var("COVEN_CODE_TEST_HOME"),
        }
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }

        assert!(files.iter().all(|file| file.scope != MemoryScope::User));
        assert!(files.iter().any(|file| {
            file.scope == MemoryScope::Project && file.content.contains("project memory")
        }));
    }

    #[test]
    fn hosted_review_loads_managed_rules_only_when_allowed() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let rules = home.path().join(".coven-code").join("rules");
        std::fs::create_dir_all(&rules).unwrap();
        std::fs::write(rules.join("managed.md"), "managed hosted policy").unwrap();

        let _lock = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let original_test_home = std::env::var("COVEN_CODE_TEST_HOME").ok();
        let original_home = std::env::var("HOME").ok();
        let original_userprofile = std::env::var("USERPROFILE").ok();
        std::env::set_var("COVEN_CODE_TEST_HOME", home.path());
        std::env::set_var("HOME", home.path());
        std::env::set_var("USERPROFILE", home.path());

        let default_hosted =
            load_all_memory_files_with_options(project.path(), &MemoryLoadOptions::hosted_review());
        let mut trusted_policy = MemoryLoadOptions::hosted_review();
        trusted_policy.allow_managed_rules = true;
        let trusted_hosted = load_all_memory_files_with_options(project.path(), &trusted_policy);

        match original_test_home {
            Some(value) => std::env::set_var("COVEN_CODE_TEST_HOME", value),
            None => std::env::remove_var("COVEN_CODE_TEST_HOME"),
        }
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }

        assert!(default_hosted
            .iter()
            .all(|file| file.scope != MemoryScope::Managed));
        assert!(trusted_hosted.iter().any(|file| {
            file.scope == MemoryScope::Managed && file.content.contains("managed hosted policy")
        }));
    }

    #[test]
    fn local_memory_load_still_includes_user_memory() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("AGENTS.md"), "project memory").unwrap();

        let home = tempfile::tempdir().unwrap();
        let coven_code = home.path().join(".coven-code");
        std::fs::create_dir_all(&coven_code).unwrap();
        std::fs::write(coven_code.join("AGENTS.md"), "user memory").unwrap();

        let _lock = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let original_test_home = std::env::var("COVEN_CODE_TEST_HOME").ok();
        let original_home = std::env::var("HOME").ok();
        let original_userprofile = std::env::var("USERPROFILE").ok();
        std::env::set_var("COVEN_CODE_TEST_HOME", home.path());
        std::env::set_var("HOME", home.path());
        std::env::set_var("USERPROFILE", home.path());

        let files = load_all_memory_files_with_options(project.path(), &MemoryLoadOptions::local());

        match original_test_home {
            Some(value) => std::env::set_var("COVEN_CODE_TEST_HOME", value),
            None => std::env::remove_var("COVEN_CODE_TEST_HOME"),
        }
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }

        assert!(files
            .iter()
            .any(|file| file.scope == MemoryScope::User && file.content.contains("user memory")));
    }

    #[test]
    fn expand_includes_circular() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.md");
        let b = tmp.path().join("b.md");
        std::fs::write(&a, "@include b.md\n").unwrap();
        std::fs::write(&b, "@include a.md\ncontent\n").unwrap();
        let result = expand_includes(
            "@include a.md\n",
            tmp.path(),
            &mut std::collections::HashSet::new(),
            0,
        );
        // Should not infinite-loop; circular reference comment present.
        assert!(result.contains("circular") || result.contains("content"));
    }
}
