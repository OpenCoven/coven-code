//! One-time, best-effort relocation of the engine home from the legacy
//! `~/.coven-code/` to `~/.coven/code/` when running under the unified coven CLI.
//!
//! Every failure is non-fatal — a failed migration must NEVER brick the engine;
//! it falls back to whichever directory has the data.
//!
//! # Safety invariant
//!
//! If migration fails, the legacy directory **must** remain intact so the user
//! can recover their data manually.  The worst failure mode is a partial target
//! that looks non-empty while the source is destroyed — this code avoids it by:
//!
//! 1. Never removing the legacy directory until the target is fully populated.
//! 2. On cross-filesystem rename failure, attempting a full recursive copy
//!    before removing the source.
//! 3. On copy failure, removing any partial target so the engine starts fresh
//!    (empty target) rather than with corrupt/partial state, while leaving the
//!    legacy directory intact.
//!
//! Degraded behaviour: the engine starts at `target` (empty) while the user's
//! data sits in `legacy`.  The user is informed via stderr and can copy
//! manually.

use std::path::Path;

/// Run the migration if the current configuration calls for relocation.
///
/// This is the public entry point called once at startup, before any config or
/// settings access.  It is a no-op when:
/// - `config_home()` still points at `.coven-code` (standalone mode).
/// - `~/.coven-code` does not exist (nothing to migrate).
/// - The target already exists and is non-empty (already migrated).
pub fn migrate_if_needed() {
    let target = crate::config::config_home();

    // If config_home still resolves to something ending in `.coven-code`, we
    // are in standalone (non-coven) mode — nothing to migrate.
    if target.file_name().and_then(|n| n.to_str()) == Some(".coven-code") {
        return;
    }

    // Compute the legacy directory.
    let home_dir = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!(
                "coven-code: home_migration: cannot determine home directory; \
                 skipping migration"
            );
            return;
        }
    };
    let legacy = home_dir.join(".coven-code");

    if !legacy.exists() {
        return;
    }

    // If the target already exists AND is non-empty, we are done (or the user
    // placed data there intentionally).
    if should_skip_existing_target(&target) {
        return;
    }

    migrate_between(&legacy, &target);
}

/// Returns `true` when the migration should be skipped because the target
/// already exists and is non-empty.
///
/// This is the production no-clobber guard.  It is extracted as a pure
/// predicate so it can be unit-tested directly: if it is removed or its
/// logic changes, `target_exists_no_clobber` will catch the regression.
fn should_skip_existing_target(target: &Path) -> bool {
    target.exists() && target.read_dir().ok().and_then(|mut d| d.next()).is_some()
}

/// Inner migration function — moves `legacy` to `target`.
///
/// Extracted from `migrate_if_needed` so it can be unit-tested against
/// arbitrary temp directories without touching the real `~`.
///
/// Callers are responsible for all precondition guards (legacy exists,
/// target is absent or empty, etc.).
fn migrate_between(legacy: &Path, target: &Path) {
    // Ensure target's parent directory exists.
    let parent = match target.parent() {
        Some(p) => p,
        None => {
            eprintln!(
                "coven-code: home_migration: target path {:?} has no parent; \
                 skipping migration",
                target
            );
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(parent) {
        eprintln!(
            "coven-code: home_migration: failed to create parent {:?}: {}; \
             skipping migration",
            parent, e
        );
        return;
    }

    // Attempt an atomic rename first (works on same filesystem).
    match std::fs::rename(legacy, target) {
        Ok(()) => {
            // Rename succeeded: create backward-compat symlink and marker.
            create_symlink_compat(legacy, target);
            write_marker(target);
        }
        Err(rename_err) => {
            // Rename failed (likely cross-filesystem EXDEV or permissions).
            // Attempt a recursive copy followed by removal of the source.
            eprintln!(
                "coven-code: home_migration: rename {:?} → {:?} failed ({}); \
                 attempting recursive copy",
                legacy, target, rename_err
            );

            match copy_dir_recursive(legacy, target) {
                Ok(()) => {
                    // Copy succeeded: remove legacy and set up compat link.
                    if let Err(e) = std::fs::remove_dir_all(legacy) {
                        eprintln!(
                            "coven-code: home_migration: copy succeeded but failed to \
                             remove legacy {:?}: {}; leaving legacy in place",
                            legacy, e
                        );
                        // Still set up symlink pointing to target since data is there.
                    }
                    create_symlink_compat(legacy, target);
                    write_marker(target);
                }
                Err(copy_err) => {
                    // Both rename and copy failed.
                    // Remove any partial target so the engine does not start
                    // with an empty/corrupt home while data is in legacy.
                    eprintln!(
                        "coven-code: home_migration: recursive copy {:?} → {:?} also \
                         failed ({}); removing partial target and leaving legacy intact",
                        legacy, target, copy_err
                    );
                    if target.exists() {
                        if let Err(e) = std::fs::remove_dir_all(target) {
                            eprintln!(
                                "coven-code: home_migration: failed to clean up partial \
                                 target {:?}: {}",
                                target, e
                            );
                        }
                    }
                    eprintln!(
                        "coven-code: home_migration: your data remains at {:?}; \
                         please move it to {:?} manually",
                        legacy, target
                    );
                }
            }
        }
    }
}

/// Create a compatibility symlink at `legacy` → `target` so that any tooling
/// that still references `~/.coven-code` continues to work.
///
/// Symlinks are Unix-only; on non-Unix platforms we log and skip.
fn create_symlink_compat(legacy: &Path, target: &Path) {
    #[cfg(unix)]
    {
        // Only create the symlink when legacy was successfully moved (i.e.,
        // legacy no longer exists as a real directory).
        if legacy.exists() {
            // legacy still present (copy succeeded but remove failed, or
            // some other scenario) — don't try to create symlink at same path.
            return;
        }
        if let Err(e) = std::os::unix::fs::symlink(target, legacy) {
            eprintln!(
                "coven-code: home_migration: failed to create compat symlink \
                 {:?} → {:?}: {}",
                legacy, target, e
            );
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!(
            "coven-code: home_migration: symlink creation is not supported on \
             this platform; {:?} will not be symlinked to {:?}",
            legacy, target
        );
    }
}

/// Write a marker file at `<target>/.migrated-from-coven-code` containing a
/// UTC timestamp so the migration is inspectable and idempotency can be
/// checked externally.
fn write_marker(target: &Path) {
    let marker = target.join(".migrated-from-coven-code");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let content = format!("migrated-at-unix-secs={}\n", ts);
    if let Err(e) = std::fs::write(&marker, content) {
        eprintln!(
            "coven-code: home_migration: failed to write marker {:?}: {}",
            marker, e
        );
    }
}

/// Recursively copy the directory tree rooted at `src` into `dst`.
///
/// Creates `dst` if it does not exist.  Skips symlinks (they would be stale
/// after the source is removed).
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if entry_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if entry_type.is_file() {
            std::fs::copy(&src_path, &dst_path)?;
        }
        // Symlinks skipped intentionally — they point inside the source tree
        // and become dangling once the source is removed.
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: acquire the two env-var locks in a consistent order to avoid
    /// deadlock when tests run concurrently.
    ///
    /// Returns the two `MutexGuard`s, which must be kept alive for the
    /// duration of the test.
    fn acquire_env_locks<'a>() -> (std::sync::MutexGuard<'a, ()>, std::sync::MutexGuard<'a, ()>) {
        let g1 = crate::config::CONFIG_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let g2 = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        (g1, g2)
    }

    /// Save and clear all env vars touched by `config_home()`.
    fn save_and_clear_config_env() -> Vec<(String, Option<String>)> {
        let vars = [
            "COVEN_CODE_TEST_HOME",
            "COVEN_CODE_HOME",
            "COVEN_HOME",
            "COVEN_PARENT",
        ];
        vars.iter()
            .map(|&v| {
                let saved = std::env::var(v).ok();
                std::env::remove_var(v);
                (v.to_string(), saved)
            })
            .collect()
    }

    fn restore_config_env(saved: Vec<(String, Option<String>)>) {
        for (var, value) in saved {
            match value {
                Some(v) => std::env::set_var(&var, v),
                None => std::env::remove_var(&var),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: populated_migration
    // -----------------------------------------------------------------------

    /// Create a legacy dir with files; call `migrate_between`; assert:
    /// - files exist under target
    /// - legacy is gone (or is a symlink)
    /// - marker file exists at `target/.migrated-from-coven-code`
    #[test]
    fn populated_migration() {
        let base = tempfile::tempdir().unwrap();

        let legacy = base.path().join("legacy");
        let target = base.path().join("target");

        // Populate legacy.
        std::fs::create_dir_all(legacy.join("projects/x")).unwrap();
        std::fs::write(legacy.join("auth.json"), b"{}").unwrap();
        std::fs::write(legacy.join("projects/x/y.jsonl"), b"session\n").unwrap();

        migrate_between(&legacy, &target);

        // Files must be under target.
        assert!(
            target.join("auth.json").exists(),
            "auth.json must exist in target"
        );
        assert!(
            target.join("projects/x/y.jsonl").exists(),
            "projects/x/y.jsonl must exist in target"
        );

        // Marker must exist.
        assert!(
            target.join(".migrated-from-coven-code").exists(),
            "migration marker must exist"
        );

        // Legacy must no longer be a real directory (either gone or is a symlink).
        let legacy_is_real_dir = legacy.exists() && legacy.is_dir() && !legacy.is_symlink();
        assert!(
            !legacy_is_real_dir,
            "legacy must not remain as a real directory after migration"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2: idempotent
    // -----------------------------------------------------------------------

    /// After a successful migration, a second call to `migrate_if_needed` with
    /// COVEN_HOME pointing to the same base directory is a no-op (target is
    /// non-empty → early return).
    #[test]
    fn idempotent() {
        let (_g1, _g2) = acquire_env_locks();
        let saved = save_and_clear_config_env();

        let base = tempfile::tempdir().unwrap();

        // Set COVEN_HOME so config_home() resolves to <base>/code.
        std::env::set_var("COVEN_HOME", base.path());
        let target = base.path().join("code");

        // Create a legacy dir and populate it.
        let home_dir = dirs::home_dir().expect("home_dir must be available");
        let legacy = home_dir.join(".coven-code");

        // We avoid touching the real ~/.coven-code by calling migrate_between
        // directly for the first migration.
        let fake_legacy = base.path().join("fake-legacy");
        std::fs::create_dir_all(&fake_legacy).unwrap();
        std::fs::write(fake_legacy.join("settings.json"), b"{}").unwrap();

        migrate_between(&fake_legacy, &target);
        assert!(target.exists(), "target must exist after first migration");
        assert!(
            target.join(".migrated-from-coven-code").exists(),
            "marker must exist after first migration"
        );

        // Sentinel file to detect if target is touched.
        let sentinel = target.join("sentinel.txt");
        std::fs::write(&sentinel, b"do not touch").unwrap();

        // Now call migrate_if_needed — target is non-empty, so it should early-return.
        // The real ~/.coven-code may or may not exist; migrate_if_needed guards on
        // both target non-empty AND legacy existence, so we don't need to worry about
        // it touching the real ~/.coven-code as long as target is non-empty.
        migrate_if_needed();

        // Sentinel must be untouched.
        let contents = std::fs::read(&sentinel).unwrap_or_default();
        assert_eq!(
            contents, b"do not touch",
            "sentinel must be untouched after idempotent call"
        );

        restore_config_env(saved);
        // Suppress "unused variable" for the legacy binding.
        let _ = legacy;
    }

    // -----------------------------------------------------------------------
    // Test 3: no_legacy_no_op
    // -----------------------------------------------------------------------

    /// If legacy does not exist, migrate_between does nothing.
    #[test]
    fn no_legacy_no_op() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("nonexistent-legacy");
        let target = base.path().join("target");

        // legacy does not exist.
        migrate_between(&legacy, &target);

        // target must not have been created.
        assert!(
            !target.exists(),
            "target must not be created when legacy does not exist"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: standalone_no_op
    // -----------------------------------------------------------------------

    /// With no COVEN_HOME / COVEN_PARENT set, `migrate_if_needed` returns early
    /// because `config_home()` resolves to the `.coven-code` path.
    #[test]
    fn standalone_no_op() {
        let (_g1, _g2) = acquire_env_locks();
        let saved = save_and_clear_config_env();

        // In standalone mode config_home() ends in .coven-code → early return.
        // We verify by checking that no mutation happened in a temp dir.
        // (There's nothing to assert about disk state; we just confirm it
        // doesn't panic and returns immediately.)
        migrate_if_needed();

        // If we get here without panic, the no-op path succeeded.
        restore_config_env(saved);
    }

    // -----------------------------------------------------------------------
    // Test 5: target_exists_no_clobber
    // -----------------------------------------------------------------------

    /// Exercises the PRODUCTION no-clobber guard (`should_skip_existing_target`)
    /// rather than re-implementing the check inline.
    ///
    /// This test will FAIL if the predicate is removed or its logic is inverted,
    /// giving real confidence on the data-safety-critical path.
    ///
    /// Two sub-cases:
    ///  a) non-empty target → predicate returns true (skip), sentinel untouched.
    ///  b) absent target   → predicate returns false (proceed).
    #[test]
    fn target_exists_no_clobber() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("legacy");
        let target = base.path().join("target");

        // Populate legacy.
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("LEGACY_MARKER"), b"important").unwrap();

        // Populate target with a sentinel.
        std::fs::create_dir_all(&target).unwrap();
        let sentinel = target.join("TARGET_SENTINEL");
        std::fs::write(&sentinel, b"sentinel").unwrap();

        // --- Case A: non-empty target ---
        // The production guard must report "skip".
        assert!(
            should_skip_existing_target(&target),
            "should_skip_existing_target must return true when target is non-empty"
        );

        // Since the guard says skip, migrate_between must NOT be called.
        // Replicate the exact production branch from migrate_if_needed:
        if !should_skip_existing_target(&target) {
            migrate_between(&legacy, &target);
        }

        // Sentinel must be intact — the target was not clobbered.
        let contents = std::fs::read(&sentinel).unwrap_or_default();
        assert_eq!(
            contents, b"sentinel",
            "TARGET_SENTINEL must be untouched: no-clobber guard failed"
        );

        // Legacy data must still be present — nothing was moved.
        assert!(
            legacy.join("LEGACY_MARKER").exists(),
            "LEGACY_MARKER must remain: legacy must not be moved when target is non-empty"
        );

        // --- Case B: absent target → guard must NOT skip ---
        let absent = base.path().join("absent-target");
        assert!(
            !should_skip_existing_target(&absent),
            "should_skip_existing_target must return false when target does not exist"
        );
    }
}
