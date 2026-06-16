//! Helpers for writing credential/secret files with owner-only permissions.
//!
//! Secret-bearing files (OAuth tokens, API keys, the provider key store) must
//! not be world-readable on shared or CI hosts. These helpers centralize the
//! "write then tighten permissions" pattern so individual call sites can't
//! forget the `chmod 0600` (see audit finding SEC-CRED-2). The parent directory
//! is created with `0700` on Unix.
//!
//! On non-Unix platforms the permission tightening is a best-effort no-op (the
//! file is still written); Windows ACL hardening is left as a follow-up.

use std::io;
use std::path::Path;

/// Set a path's mode to `mode` (Unix only; no-op elsewhere). Best-effort.
#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

/// Create `dir` (and parents) and, on Unix, tighten it to `0700`.
fn ensure_secret_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // Only tighten the leaf dir; parents may legitimately be shared (e.g. the
    // user's home). Ignore errors from set_mode beyond surfacing them.
    set_mode(dir, 0o700)
}

/// Synchronously write `contents` to `path` as an owner-only (`0600`) file,
/// creating the parent directory (`0700`) if needed.
///
/// The file is created/truncated, written, then `chmod`-ed. If the file already
/// existed with looser permissions, this resets it to `0600`.
pub fn write_secret_file_sync(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            ensure_secret_dir(parent)?;
        }
    }
    std::fs::write(path, contents)?;
    set_mode(path, 0o600)
}

/// Async variant of [`write_secret_file_sync`] using `tokio::fs`.
pub async fn write_secret_file(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
            // set_permissions is sync in std; the dir already exists so this is cheap.
            set_mode(parent, 0o700)?;
        }
    }
    tokio::fs::write(path, contents).await?;
    set_mode(path, 0o600)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        // Unique-enough per test name; avoids pulling in a tempdir dep.
        let mut dir = std::env::temp_dir();
        dir.push(format!("coven-secret-file-test-{name}"));
        // Clean any leftover from a prior run.
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn sync_writes_contents_and_tightens_mode() {
        let dir = temp_path("sync");
        let path = dir.join("nested").join("secret.json");
        write_secret_file_sync(&path, b"{\"token\":\"abc\"}").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"{\"token\":\"abc\"}");
        #[cfg(unix)]
        {
            assert_eq!(mode_of(&path), 0o600, "file should be owner-only");
            assert_eq!(
                mode_of(&path.parent().unwrap()),
                0o700,
                "parent dir should be owner-only"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_resets_loose_mode_on_existing_file() {
        let dir = temp_path("reset");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secret.json");
        std::fs::write(&path, b"old").unwrap();
        #[cfg(unix)]
        set_mode(&path, 0o644).unwrap();

        write_secret_file_sync(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        #[cfg(unix)]
        assert_eq!(mode_of(&path), 0o600, "mode should be re-tightened to 0600");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn async_writes_contents_and_tightens_mode() {
        let dir = temp_path("async");
        let path = dir.join("secret.json");
        write_secret_file(&path, b"async-secret").await.unwrap();

        assert_eq!(
            tokio::fs::read(&path).await.unwrap(),
            b"async-secret".to_vec()
        );
        #[cfg(unix)]
        assert_eq!(mode_of(&path), 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
