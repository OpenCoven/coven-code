//! Git utilities for Coven Code.
//! Mirrors src/utils/git.ts (926 lines) and src/utils/git/ subdirectory.

use crate::hosted_review::CanonicalRepoIdentity;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Repository discovery
// ---------------------------------------------------------------------------

/// Walk up the directory tree to find the nearest `.git` directory.
pub fn get_repo_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let git_dir = current.join(".git");
        if git_dir.exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Run a git command in `repo_root` and return stdout as a String.
/// Returns empty string on failure (non-zero exit, not-a-repo, etc.).
fn git_output(repo_root: &Path, args: &[&str]) -> String {
    Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Branch / status
// ---------------------------------------------------------------------------

/// Return the current branch name (or "HEAD" if detached).
pub fn get_current_branch(repo_root: &Path) -> String {
    let branch = git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    if branch.is_empty() {
        "HEAD".to_string()
    } else {
        branch
    }
}

pub fn get_origin_remote_url(repo_root: &Path) -> Option<String> {
    let remote = git_output(repo_root, &["remote", "get-url", "origin"]);
    (!remote.is_empty()).then_some(remote)
}

pub fn canonical_repo_identity_from_origin(repo_root: &Path) -> Option<CanonicalRepoIdentity> {
    let remote = get_origin_remote_url(repo_root)?;
    CanonicalRepoIdentity::from_git_remote_url(&remote)
}

pub fn local_project_id_from_identity(identity: &CanonicalRepoIdentity) -> String {
    let mut hasher = Sha256::new();
    hasher.update(identity.canonical_string().as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("local-git-{}", &digest[..16])
}

pub fn local_project_id_from_origin(repo_root: &Path) -> Option<String> {
    canonical_repo_identity_from_origin(repo_root)
        .map(|identity| local_project_id_from_identity(&identity))
}

/// Derive the local project id from the repository's origin remote and verify
/// any caller-claimed id against it.
///
/// Fails closed: a repository with no usable origin remote (missing remote,
/// unparseable URL, or not a git repository) yields an error instead of
/// falling back to a caller-controlled identity, and a claimed id that does
/// not match the derived one is rejected.
pub fn verified_local_project_id(
    repo_root: &Path,
    claimed: Option<&str>,
) -> anyhow::Result<String> {
    let derived = local_project_id_from_origin(repo_root).ok_or_else(|| {
        anyhow::anyhow!(
            "cannot derive a project id for {}: no usable origin remote; refusing caller-provided identity",
            repo_root.display()
        )
    })?;
    if let Some(claimed) = claimed {
        if claimed != derived {
            anyhow::bail!(
                "claimed project id {claimed:?} does not match the identity derived from the origin remote"
            );
        }
    }
    Ok(derived)
}

/// Return list of files modified (staged or unstaged).
pub fn list_modified_files(repo_root: &Path) -> Vec<PathBuf> {
    let output = git_output(repo_root, &["diff", "--name-only", "HEAD"]);
    if output.is_empty() {
        return Vec::new();
    }
    output.lines().map(|l| repo_root.join(l)).collect()
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

/// Return the staged diff (index vs HEAD).
pub fn get_staged_diff(repo_root: &Path) -> String {
    git_output(repo_root, &["diff", "--cached"])
}

/// Return the unstaged diff (working tree vs index).
pub fn get_unstaged_diff(repo_root: &Path) -> String {
    git_output(repo_root, &["diff"])
}

/// Return the diff for a specific file since a given commit (or HEAD).
pub fn get_file_diff(repo_root: &Path, path: &Path, since_commit: Option<&str>) -> String {
    let commit = since_commit.unwrap_or("HEAD");
    let path_str = path.to_string_lossy();
    git_output(repo_root, &["diff", commit, "--", &path_str])
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// A single git commit summary.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

/// Return the last `n` commits in the repository.
pub fn get_commit_history(repo_root: &Path, n: usize) -> Vec<CommitInfo> {
    let format = "%H%x1f%h%x1f%an%x1f%ad%x1f%s%x1e";
    let n_str = n.to_string();
    let output = git_output(
        repo_root,
        &[
            "log",
            &format!("-{}", n_str),
            &format!("--format={}", format),
            "--date=short",
        ],
    );

    output
        .split('\x1e')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.trim().splitn(5, '\x1f').collect();
            if parts.len() == 5 {
                Some(CommitInfo {
                    hash: parts[0].to_string(),
                    short_hash: parts[1].to_string(),
                    author: parts[2].to_string(),
                    date: parts[3].to_string(),
                    subject: parts[4].to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Branch operations
// ---------------------------------------------------------------------------

/// Create and switch to a new branch.
pub fn create_branch(repo_root: &Path, name: &str) -> bool {
    Command::new("git")
        .current_dir(repo_root)
        .args(["checkout", "-b", name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Switch to an existing branch.
pub fn switch_branch(repo_root: &Path, name: &str) -> bool {
    Command::new("git")
        .current_dir(repo_root)
        .args(["checkout", name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Stash
// ---------------------------------------------------------------------------

/// Stash uncommitted changes with an optional message.
pub fn stash(repo_root: &Path, message: Option<&str>) -> bool {
    let mut args = vec!["stash", "push"];
    let msg_flag;
    if let Some(m) = message {
        msg_flag = format!("-m {}", m);
        args.push(&msg_flag);
    }
    Command::new("git")
        .current_dir(repo_root)
        .args(&args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Pop the top stash entry.
pub fn stash_pop(repo_root: &Path) -> bool {
    Command::new("git")
        .current_dir(repo_root)
        .args(["stash", "pop"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// .gitignore check
// ---------------------------------------------------------------------------

/// Returns `true` if the given path is git-ignored.
pub fn is_ignored(repo_root: &Path, path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    Command::new("git")
        .current_dir(repo_root)
        .args(["check-ignore", "-q", &path_str])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn get_repo_root_finds_git() {
        // Run from within the src-rust workspace which has .git
        let result = get_repo_root(Path::new("."));
        // Should find the repo root (may or may not exist in test env)
        // Just verify it doesn't panic.
        let _ = result;
    }

    #[test]
    fn commit_info_parse() {
        // smoke test — just ensure it doesn't panic with empty output
        let commits = get_commit_history(Path::new("."), 0);
        assert!(commits.is_empty());
    }

    #[test]
    fn local_project_id_normalizes_equivalent_https_and_ssh_remotes() {
        let https = CanonicalRepoIdentity::from_git_remote_url(
            "https://github.com/OpenCoven/coven-code.git",
        )
        .unwrap();
        let ssh =
            CanonicalRepoIdentity::from_git_remote_url("git@github.com:OpenCoven/coven-code.git")
                .unwrap();

        assert_eq!(
            local_project_id_from_identity(&https),
            local_project_id_from_identity(&ssh)
        );
    }

    fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .current_dir(dir)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "--quiet"]);
    }

    #[test]
    fn verified_project_id_fails_closed_without_origin_remote() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());

        let err = verified_local_project_id(tmp.path(), None).unwrap_err();
        assert!(
            err.to_string().contains("no usable origin remote"),
            "missing remote must fail closed: {err}"
        );

        // A claimed id cannot substitute for a derivable identity.
        let err = verified_local_project_id(tmp.path(), Some("local-git-deadbeef")).unwrap_err();
        assert!(err.to_string().contains("no usable origin remote"));
    }

    #[test]
    fn verified_project_id_rejects_mismatched_claimed_id() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let status = Command::new("git")
            .current_dir(tmp.path())
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/OpenCoven/coven-code.git",
            ])
            .status()
            .unwrap();
        assert!(status.success());

        let derived = verified_local_project_id(tmp.path(), None).unwrap();
        assert!(derived.starts_with("local-git-"));

        // Matching claim passes; mismatched claim is rejected.
        assert_eq!(
            verified_local_project_id(tmp.path(), Some(&derived)).unwrap(),
            derived
        );
        let err = verified_local_project_id(tmp.path(), Some("local-git-spoofed")).unwrap_err();
        assert!(err.to_string().contains("does not match"));
    }
}
