//! Helpers for writing credential/secret files with owner-only permissions.
//!
//! Secret-bearing files (OAuth tokens, API keys, the provider key store) must
//! not be world-readable on shared or CI hosts. These helpers create a private
//! same-directory tempfile before writing any credential bytes, then atomically
//! replace the destination (see audit finding SEC-CRED-2).
//!
//! Unix creation modes are enforced here. Native Windows ACL hardening remains
//! a separate follow-up.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Set a path's mode to `mode` (Unix only; no-op elsewhere).
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

/// Create or tighten the exact credential directory without changing any
/// ancestor. A symlink is rejected so credential writes cannot escape through
/// an attacker-controlled directory leaf.
fn ensure_secret_dir(dir: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("credential directory is a symlink: {}", dir.display()),
                ));
            }
            if !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    format!("credential directory is not a directory: {}", dir.display()),
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = std::fs::DirBuilder::new();
                builder.mode(0o700);
                match builder.create(dir) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        return ensure_secret_dir(dir);
                    }
                    Err(error) => return Err(error),
                }
            }
            #[cfg(not(unix))]
            match std::fs::create_dir(dir) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return ensure_secret_dir(dir);
                }
                Err(error) => return Err(error),
            }
        }
        Err(error) => return Err(error),
    }
    set_mode(dir, 0o700)
}

fn prepare_parent(path: &Path, private_parent: bool) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            if private_parent {
                ensure_secret_dir(parent)?;
            } else {
                std::fs::create_dir_all(parent)?;
            }
        }
    }
    Ok(())
}

struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn temp_name(path: &Path, suffix: &str) -> OsString {
    let file_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("secret"))
        .to_string_lossy();
    format!(".{file_name}.tmp.{suffix}").into()
}

fn write_secret_file_sync_with_names<I, S, F>(
    path: &Path,
    contents: &[u8],
    private_parent: bool,
    suffixes: I,
    before_rename: F,
) -> io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    F: FnOnce(&Path) -> io::Result<()>,
{
    prepare_parent(path, private_parent)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut opened = None;

    for suffix in suffixes {
        let candidate = parent.join(temp_name(path, suffix.as_ref()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => {
                opened = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    let (temp_path, mut file) = opened.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique credential tempfile",
        )
    })?;
    let mut guard = TempFileGuard {
        path: temp_path,
        armed: true,
    };
    file.write_all(contents)?;
    file.flush()?;
    before_rename(&guard.path)?;
    drop(file);
    std::fs::rename(&guard.path, path)?;
    guard.armed = false;
    Ok(())
}

fn write_secret_file_sync_inner(
    path: &Path,
    contents: &[u8],
    private_parent: bool,
) -> io::Result<()> {
    let suffixes = std::iter::repeat_with(|| uuid::Uuid::new_v4().simple().to_string()).take(128);
    write_secret_file_sync_with_names(path, contents, private_parent, suffixes, |_| Ok(()))
}

/// Atomically write an owner-only (`0600` on Unix) secret file. The parent is
/// created when absent but its permissions are not changed.
pub fn write_secret_file_sync(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    write_secret_file_sync_inner(path, contents.as_ref(), false)
}

/// Atomically write a secret and create/tighten its exact credential directory
/// to `0700` on Unix. Ancestor permissions are left unchanged.
pub fn write_secret_file_sync_in_private_dir(
    path: &Path,
    contents: impl AsRef<[u8]>,
) -> io::Result<()> {
    write_secret_file_sync_inner(path, contents.as_ref(), true)
}

async fn write_secret_file_inner(
    path: &Path,
    contents: impl AsRef<[u8]>,
    private_parent: bool,
) -> io::Result<()> {
    let path = path.to_path_buf();
    let contents = contents.as_ref().to_vec();
    tokio::task::spawn_blocking(move || {
        write_secret_file_sync_inner(&path, &contents, private_parent)
    })
    .await
    .map_err(io::Error::other)?
}

/// Async wrapper for [`write_secret_file_sync`].
pub async fn write_secret_file(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    write_secret_file_inner(path, contents, false).await
}

/// Async wrapper for [`write_secret_file_sync_in_private_dir`].
pub async fn write_secret_file_in_private_dir(
    path: &Path,
    contents: impl AsRef<[u8]>,
) -> io::Result<()> {
    write_secret_file_inner(path, contents, true).await
}

#[cfg(test)]
fn write_secret_file_sync_for_test<I, S, F>(
    path: &Path,
    contents: impl AsRef<[u8]>,
    private_parent: bool,
    suffixes: I,
    before_rename: F,
) -> io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    F: FnOnce(&Path) -> io::Result<()>,
{
    write_secret_file_sync_with_names(
        path,
        contents.as_ref(),
        private_parent,
        suffixes,
        before_rename,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn sync_writes_contents_and_tightens_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("secret.json");
        write_secret_file_sync_in_private_dir(&path, b"{\"token\":\"abc\"}").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"{\"token\":\"abc\"}");
        #[cfg(unix)]
        {
            assert_eq!(mode_of(&path), 0o600, "file should be owner-only");
            assert_eq!(
                mode_of(path.parent().unwrap()),
                0o700,
                "parent dir should be owner-only"
            );
        }
    }

    #[test]
    fn sync_resets_loose_mode_on_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.json");
        std::fs::write(&path, b"old").unwrap();
        #[cfg(unix)]
        set_mode(&path, 0o644).unwrap();

        write_secret_file_sync(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        #[cfg(unix)]
        assert_eq!(mode_of(&path), 0o600, "mode should be re-tightened to 0600");
    }

    #[tokio::test]
    async fn async_writes_contents_and_tightens_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.json");
        write_secret_file(&path, b"async-secret").await.unwrap();

        assert_eq!(
            tokio::fs::read(&path).await.unwrap(),
            b"async-secret".to_vec()
        );
        #[cfg(unix)]
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn atomic_failure_preserves_existing_secret_and_cleans_owned_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.json");
        std::fs::write(&path, b"old-secret").unwrap();
        std::fs::write(dir.path().join(".secret.json.tmp.collision"), b"foreign").unwrap();

        let error = write_secret_file_sync_for_test(
            &path,
            b"new-secret",
            false,
            ["collision", "owned-temp"],
            |temp_path| {
                assert_eq!(std::fs::read(temp_path).unwrap(), b"new-secret");
                #[cfg(unix)]
                assert_eq!(mode_of(temp_path), 0o600);
                Err(io::Error::other("injected before rename"))
            },
        )
        .expect_err("injected failure must propagate");

        assert_eq!(error.to_string(), "injected before rename");
        assert_eq!(std::fs::read(&path).unwrap(), b"old-secret");
        assert_eq!(
            std::fs::read(dir.path().join(".secret.json.tmp.collision")).unwrap(),
            b"foreign"
        );
        assert!(!dir.path().join(".secret.json.tmp.owned-temp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn private_directory_is_created_privately_without_chmodding_ancestor() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let credential_dir = dir.path().join("mcp-tokens");
        let path = credential_dir.join("fixture.json");

        write_secret_file_sync_in_private_dir(&path, b"fixture").unwrap();

        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(mode_of(&credential_dir), 0o700);
        assert_eq!(mode_of(dir.path()), 0o755);
    }

    #[test]
    fn concurrent_writes_leave_one_complete_payload() {
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("secret.json"));
        let payloads: Vec<Vec<u8>> = (0..32)
            .map(|index| format!("payload-{index:02}-{}", "x".repeat(4096)).into_bytes())
            .collect();
        let handles: Vec<_> = payloads
            .iter()
            .cloned()
            .map(|payload| {
                let path = Arc::clone(&path);
                std::thread::spawn(move || write_secret_file_sync(&path, payload))
            })
            .collect();

        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        let actual = std::fs::read(&*path).unwrap();
        assert!(payloads.contains(&actual));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn private_directory_symlink_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let credential_dir = dir.path().join("mcp-tokens");
        symlink(target.path(), &credential_dir).unwrap();

        let error =
            write_secret_file_sync_in_private_dir(&credential_dir.join("fixture.json"), b"fixture")
                .expect_err("credential-directory symlink must be rejected");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!target.path().join("fixture.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn destination_symlink_is_replaced_without_following_it() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.json");
        let destination = dir.path().join("secret.json");
        std::fs::write(&target, b"target-data").unwrap();
        symlink(&target, &destination).unwrap();

        write_secret_file_sync(&destination, b"replacement").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"target-data");
        assert_eq!(std::fs::read(&destination).unwrap(), b"replacement");
        assert!(!std::fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn umask_022_subprocess_proves_private_initial_creation() {
        if std::env::var_os("COVEN_SECRET_UMASK_CHILD").is_some() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("credentials").join("secret.json");
            write_secret_file_sync_for_test(&path, b"fixture", true, ["owned"], |temp_path| {
                assert_eq!(mode_of(temp_path), 0o600);
                assert_eq!(mode_of(temp_path.parent().unwrap()), 0o700);
                Ok(())
            })
            .unwrap();
            assert_eq!(mode_of(&path), 0o600);
            return;
        }

        let executable = std::env::current_exe().unwrap();
        let output = std::process::Command::new("sh")
            .args(["-c", "umask 022; exec \"$@\"", "sh"])
            .arg(executable)
            .args([
                "--exact",
                "secret_file::tests::umask_022_subprocess_proves_private_initial_creation",
                "--nocapture",
            ])
            .env("COVEN_SECRET_UMASK_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
