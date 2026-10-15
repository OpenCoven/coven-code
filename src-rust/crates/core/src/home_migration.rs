//! Best-effort relocation from legacy `~/.coven-code` to the managed engine home.
//!
//! Only typed managed-home requests are eligible. Existing destination entries
//! are never merged, replaced, or removed, even when empty. Relocation uses only
//! an atomic no-replace rename. If that cannot complete (including across file
//! systems), migration is deferred without copying or deleting legacy data.
//!
//! Config readers keep using usable legacy data while the managed target is
//! absent or unusable. An independently populated usable target keeps precedence.

use crate::config::{ConfigHomeSource, RequestedConfigHome};
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

const MARKER: &str = ".migrated-from-coven-code";

/// Run before settings/auth access. Explicit and test overrides never relocate.
pub fn migrate_if_needed() {
    migrate_requested(&crate::config::requested_config_home(), &RealFs);
}

/// Listing and child lookup must both work: listing alone does not prove
/// directory search permission. lstat keeps dangling child links as user data.
pub(crate) fn has_home_data(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some_and(|entry| entry.is_ok_and(|entry| fs::symlink_metadata(entry.path()).is_ok()))
}

#[derive(Debug, PartialEq, Eq)]
enum MigrationOutcome {
    NotNeeded,
    RetainedLegacy,
    Published,
}

/// Private filesystem seam: tests force failures at the actual owning operations.
trait MigrationFs {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        rename_noreplace(from, to)
    }
}
struct RealFs;
impl MigrationFs for RealFs {}

fn migrate_requested(request: &RequestedConfigHome, ops: &impl MigrationFs) -> MigrationOutcome {
    if request.source != ConfigHomeSource::Managed {
        return MigrationOutcome::NotNeeded;
    }
    let Some(legacy) = &request.legacy else {
        return MigrationOutcome::NotNeeded;
    };
    // Never relocate a root symlink, including a previously installed compat link.
    let source_identity = match DirectoryIdentity::open(legacy) {
        Ok(identity) => identity,
        Err(_) => return MigrationOutcome::NotNeeded,
    };
    // symlink_metadata sees dangling links too. Inspection errors fail closed.
    match fs::symlink_metadata(&request.path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        _ => return MigrationOutcome::NotNeeded,
    }
    let result = relocate(legacy, &request.path, source_identity, ops);
    match result {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("coven-code: home_migration: automatic migration deferred from {legacy:?} to {:?}: {error}; legacy data retained", request.path);
            MigrationOutcome::RetainedLegacy
        }
    }
}

fn relocate(
    legacy: &Path,
    requested_target: &Path,
    source_identity: DirectoryIdentity,
    ops: &impl MigrationFs,
) -> io::Result<MigrationOutcome> {
    // Validate the initial no-follow observation before adopting any resolved
    // path or creating parents. A replacement root is never ours to relocate.
    source_identity.verify(legacy)?;
    let source = fs::canonicalize(legacy)?;
    source_identity.verify(&source)?;
    let target = resolve_missing_path(requested_target)?;
    // Check aliases/ancestry before creating target parents.
    if source.starts_with(&target) || target.starts_with(&source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source and target overlap",
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "target has no parent"))?;
    fs::create_dir_all(parent)?;
    source_identity.verify(legacy)?;
    source_identity.verify(&source)?;
    // Copying followed by recursive removal cannot preserve writes from another
    // process using legacy. A failed atomic move therefore leaves it in place.
    ops.rename(&source, &target)?;
    create_symlink_compat(legacy, &target);
    write_marker(&target);
    Ok(MigrationOutcome::Published)
}

/// Resolve existing symlink ancestors while allowing a not-yet-created suffix.
/// Parent components are processed after symlink expansion, as the OS does.
fn resolve_missing_path(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => {
                resolved.push(other.as_os_str());
                match fs::symlink_metadata(&resolved) {
                    Ok(_) => resolved = fs::canonicalize(&resolved)?,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok(resolved)
}

/// A no-follow open pins the observed directory while comparing identities.
/// Replacement roots must not be adopted or relocated by this attempt.
struct DirectoryIdentity {
    id: (u64, u64),
    _handle: fs::File,
}

impl DirectoryIdentity {
    fn open(path: &Path) -> io::Result<Self> {
        #[cfg(any(
            target_vendor = "apple",
            target_os = "linux",
            target_os = "android",
            target_os = "redox"
        ))]
        {
            use std::os::unix::fs::MetadataExt;
            let handle = fs::File::from(rustix::fs::open(
                path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )?);
            let metadata = handle.metadata()?;
            Ok(Self {
                id: (metadata.dev(), metadata.ino()),
                _handle: handle,
            })
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
                FILE_FLAG_OPEN_REPARSE_POINT,
            };
            let handle = fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)?;
            let metadata = handle.metadata()?;
            if !metadata.is_dir() || metadata.is_symlink() {
                return Err(io::Error::other("source is not a real directory"));
            }
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            // SAFETY: handle owns a live handle and info is a writable, correctly
            // sized output structure for the entire synchronous call.
            if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                id: (
                    u64::from(info.dwVolumeSerialNumber),
                    (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
                ),
                _handle: handle,
            })
        }
        #[cfg(not(any(
            target_vendor = "apple",
            target_os = "linux",
            target_os = "android",
            target_os = "redox",
            windows
        )))]
        {
            let _ = path;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no-follow directory identity unavailable",
            ))
        }
    }

    fn verify(&self, path: &Path) -> io::Result<()> {
        if Self::open(path)?.id == self.id {
            Ok(())
        } else {
            Err(io::Error::other("directory identity changed"))
        }
    }
}

fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(any(
        target_vendor = "apple",
        target_os = "linux",
        target_os = "android",
        target_os = "redox"
    ))]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            from,
            rustix::fs::CWD,
            to,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(Into::into)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut from: Vec<u16> = from.as_os_str().encode_wide().collect();
        let mut to: Vec<u16> = to.as_os_str().encode_wide().collect();
        if from.contains(&0) || to.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains NUL",
            ));
        }
        from.push(0);
        to.push(0);
        // SAFETY: both buffers are NUL-terminated, contain no embedded NUL, and
        // remain alive for the call. Flags=0 forbids destination replacement.
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(any(
        target_vendor = "apple",
        target_os = "linux",
        target_os = "android",
        target_os = "redox",
        windows
    )))]
    {
        let _ = (from, to);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic no-replace rename unavailable",
        ))
    }
}

fn create_symlink_compat(legacy: &Path, target: &Path) {
    #[cfg(unix)]
    {
        // symlink itself is exclusive; never overwrite an entry, dangling or not.
        if let Err(error) = std::os::unix::fs::symlink(target, legacy) {
            if error.kind() != io::ErrorKind::AlreadyExists {
                eprintln!("coven-code: home_migration: cannot create compatibility link {legacy:?}: {error}");
            }
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!("coven-code: home_migration: compatibility symlink skipped on this platform: {legacy:?} -> {target:?}");
    }
}

fn write_marker(target: &Path) {
    let marker = target.join(MARKER);
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_else(|_| "unknown".into());
        writeln!(file, "migrated-at-unix-secs={timestamp}")
    })();
    if let Err(error) = result {
        if error.kind() != io::ErrorKind::AlreadyExists {
            eprintln!("coven-code: home_migration: cannot write marker {marker:?}: {error}");
        }
    }
}

#[cfg(test)]
#[path = "home_migration_tests.rs"]
mod tests;
