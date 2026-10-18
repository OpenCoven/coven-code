use super::*;
use crate::config::{
    resolve_config_home, with_requested_home_for_test, ConfigHomeSource, Settings,
};
use std::cell::Cell;
use std::fs;

struct Fixture {
    base: tempfile::TempDir,
    request: crate::config::RequestedConfigHome,
}

impl Fixture {
    fn new() -> Self {
        let base = tempfile::tempdir().unwrap();
        let request = resolve_config_home(
            Some(base.path()),
            None,
            None,
            Some(base.path().join("managed").to_str().unwrap()),
            None,
        );
        let legacy = request.legacy.as_ref().unwrap();
        fs::create_dir_all(legacy.join("sessions")).unwrap();
        fs::create_dir_all(legacy.join("skills/nested/empty")).unwrap();
        fs::write(
            legacy.join("settings.json"),
            br#"{"config":{"model":"retained-model"}}"#,
        )
        .unwrap();
        fs::write(
            legacy.join("auth.json"),
            br#"{"credentials":{"anthropic":{"type":"api","key":"fixture-key"}}}"#,
        )
        .unwrap();
        fs::write(legacy.join("skills/nested/skill.md"), b"retained skill\n").unwrap();
        let mut session = crate::history::ConversationSession::new("retained-model".into());
        session.id = "retained-session".into();
        session.add_message(crate::Message::user("retained conversation"));
        fs::write(
            legacy.join("sessions/retained-session.json"),
            serde_json::to_vec(&session).unwrap(),
        )
        .unwrap();
        Self { base, request }
    }

    fn legacy(&self) -> &Path {
        self.request.legacy.as_deref().unwrap()
    }

    fn assert_consumers(&self, expected: &Path) {
        with_requested_home_for_test(self.request.clone(), || {
            assert_eq!(crate::config::config_home(), expected);
            assert_eq!(
                Settings::load_sync().unwrap().config.model.as_deref(),
                Some("retained-model")
            );
            assert_eq!(
                crate::AuthStore::load().api_key_for("anthropic").as_deref(),
                Some("fixture-key")
            );
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let session = runtime
                .block_on(crate::history::load_session("retained-session"))
                .unwrap();
            assert_eq!(
                session.messages[0].get_text(),
                Some("retained conversation")
            );
            assert_eq!(
                fs::read(crate::config::config_home().join("skills/nested/skill.md")).unwrap(),
                b"retained skill\n"
            );
        });
    }

    #[cfg(unix)]
    fn links_and_modes(&self) {
        use std::os::unix::fs::{symlink, PermissionsExt};
        for (name, value) in [
            ("relative", PathBuf::from("auth.json")),
            ("absolute", self.base.path().join("outside")),
            ("dangling", PathBuf::from("missing")),
        ] {
            symlink(value, self.legacy().join(name)).unwrap();
        }
        fs::set_permissions(self.legacy(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(
            self.legacy().join("auth.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::set_permissions(
            self.legacy().join("skills/nested"),
            fs::Permissions::from_mode(0o750),
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn assert_links_and_modes(&self, home: &Path) {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::read_link(home.join("relative")).unwrap(),
            Path::new("auth.json")
        );
        assert_eq!(
            fs::read_link(home.join("absolute")).unwrap(),
            self.base.path().join("outside")
        );
        assert_eq!(
            fs::read_link(home.join("dangling")).unwrap(),
            Path::new("missing")
        );
        assert_eq!(
            fs::metadata(home).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(home.join("auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(home.join("skills/nested"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o750
        );
        assert!(home.join("skills/nested/empty").is_dir());
    }
}

#[derive(Debug, PartialEq, Eq)]
enum EntrySnapshot {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
    Other,
}

fn snapshot(path: &Path) -> std::collections::BTreeMap<PathBuf, EntrySnapshot> {
    fn visit(
        root: &Path,
        path: &Path,
        result: &mut std::collections::BTreeMap<PathBuf, EntrySnapshot>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            let value = if kind.is_symlink() {
                EntrySnapshot::Symlink(fs::read_link(entry.path()).unwrap())
            } else if kind.is_dir() {
                visit(root, &entry.path(), result);
                EntrySnapshot::Directory
            } else if kind.is_file() {
                EntrySnapshot::File(fs::read(entry.path()).unwrap())
            } else {
                EntrySnapshot::Other
            };
            result.insert(
                entry.path().strip_prefix(root).unwrap().to_path_buf(),
                value,
            );
        }
    }
    let mut result = std::collections::BTreeMap::new();
    visit(path, path, &mut result);
    result
}

#[derive(Default)]
struct Faults {
    rename_calls: Cell<usize>,
    rename_error: Option<io::ErrorKind>,
    race_target: bool,
    race_populated: bool,
    race_file: bool,
}

impl MigrationFs for Faults {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.rename_calls.set(self.rename_calls.get() + 1);
        if let Some(kind) = self.rename_error {
            return Err(io::Error::from(kind));
        }
        if self.race_target {
            if self.race_file {
                fs::write(to, b"racing-file")?;
            } else {
                fs::create_dir(to)?;
                if self.race_populated {
                    fs::write(to.join("settings.json"), b"racing-target")?;
                }
            }
        }
        rename_noreplace(from, to)
    }
}

#[test]
fn rename_preserves_populated_home_and_second_startup() {
    let fixture = Fixture::new();
    #[cfg(unix)]
    fixture.links_and_modes();
    let result = migrate_requested(&fixture.request, &RealFs);
    assert_eq!(result, MigrationOutcome::Published);
    fixture.assert_consumers(&fixture.request.path);
    #[cfg(unix)]
    {
        fixture.assert_links_and_modes(&fixture.request.path);
        assert_eq!(
            fs::canonicalize(fixture.legacy()).unwrap(),
            fs::canonicalize(&fixture.request.path).unwrap()
        );
    }
    let marker = fs::read(fixture.request.path.join(MARKER)).unwrap();
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::NotNeeded
    );
    assert_eq!(fs::read(fixture.request.path.join(MARKER)).unwrap(), marker);
    fixture.assert_consumers(&fixture.request.path);
}

#[cfg(unix)]
#[test]
fn unsupported_entry_survives_deferred_relocation() {
    let fixture = Fixture::new();
    fixture.links_and_modes();
    let _socket = std::os::unix::net::UnixListener::bind(fixture.legacy().join("socket")).unwrap();
    let original = snapshot(fixture.legacy());
    let faults = Faults {
        rename_error: Some(io::ErrorKind::CrossesDevices),
        ..Default::default()
    };
    assert_eq!(
        migrate_requested(&fixture.request, &faults),
        MigrationOutcome::RetainedLegacy
    );
    assert_eq!(snapshot(fixture.legacy()), original);
    assert!(fs::symlink_metadata(fixture.legacy().join("socket")).is_ok());
    fixture.assert_consumers(fixture.legacy());
    fixture.assert_links_and_modes(fixture.legacy());
    assert!(!fixture.request.path.exists());
}

#[cfg(unix)]
#[test]
fn same_filesystem_rename_preserves_socket() {
    let fixture = Fixture::new();
    let _socket = std::os::unix::net::UnixListener::bind(fixture.legacy().join("socket")).unwrap();
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::Published
    );
    assert!(fs::symlink_metadata(fixture.request.path.join("socket")).is_ok());
    fixture.assert_consumers(&fixture.request.path);
}

#[test]
fn existing_target_is_never_merged_or_removed() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.request.path).unwrap();
    fs::write(fixture.request.path.join("settings.json"), b"target-data").unwrap();
    let original = fs::read(fixture.legacy().join("settings.json")).unwrap();
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::NotNeeded
    );
    assert_eq!(
        fs::read(fixture.request.path.join("settings.json")).unwrap(),
        b"target-data"
    );
    assert_eq!(
        fs::read(fixture.legacy().join("settings.json")).unwrap(),
        original
    );
    assert_eq!(fixture.request.effective_path(), fixture.request.path);
}

#[test]
fn existing_empty_or_file_target_preserves_entry_and_uses_legacy() {
    for directory in [false, true] {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.request.path.parent().unwrap()).unwrap();
        if directory {
            fs::create_dir(&fixture.request.path).unwrap();
        } else {
            fs::write(&fixture.request.path, b"unowned-file").unwrap();
        }
        assert_eq!(
            migrate_requested(&fixture.request, &RealFs),
            MigrationOutcome::NotNeeded
        );
        fixture.assert_consumers(fixture.legacy());
        if directory {
            assert_eq!(fs::read_dir(&fixture.request.path).unwrap().count(), 0);
        } else {
            assert_eq!(fs::read(&fixture.request.path).unwrap(), b"unowned-file");
        }
    }
}

#[test]
fn missing_source_does_not_create_home() {
    let fixture = Fixture::new();
    fs::remove_dir_all(fixture.legacy()).unwrap();
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::NotNeeded
    );
    assert!(!fixture.request.path.exists());
    assert_eq!(fixture.request.effective_path(), fixture.request.path);
}

#[test]
fn destination_race_does_not_replace_even_empty_directory() {
    for (race_populated, race_file) in [(false, false), (true, false), (false, true)] {
        let fixture = Fixture::new();
        let original = snapshot(fixture.legacy());
        let faults = Faults {
            race_target: true,
            race_populated,
            race_file,
            ..Default::default()
        };
        assert_eq!(
            migrate_requested(&fixture.request, &faults),
            MigrationOutcome::RetainedLegacy
        );
        assert_eq!(faults.rename_calls.get(), 1);
        assert_eq!(snapshot(fixture.legacy()), original);
        if race_populated {
            assert_eq!(
                fs::read(fixture.request.path.join("settings.json")).unwrap(),
                b"racing-target"
            );
            assert_eq!(fixture.request.effective_path(), fixture.request.path);
        } else {
            if race_file {
                assert_eq!(fs::read(&fixture.request.path).unwrap(), b"racing-file");
            } else {
                assert_eq!(fs::read_dir(&fixture.request.path).unwrap().count(), 0);
            }
            fixture.assert_consumers(fixture.legacy());
        }
    }
}

#[cfg(unix)]
#[test]
fn symlink_targets_and_aliases_are_no_ops() {
    for payload in ["legacy", "dangling", "independent"] {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.request.path.parent().unwrap()).unwrap();
        let link = match payload {
            "legacy" => fixture.legacy().to_path_buf(),
            "dangling" => fixture.base.path().join("absent"),
            _ => {
                let p = fixture.base.path().join("independent");
                fs::create_dir(&p).unwrap();
                fs::write(p.join("settings.json"), b"independent-target").unwrap();
                p
            }
        };
        std::os::unix::fs::symlink(&link, &fixture.request.path).unwrap();
        assert_eq!(
            migrate_requested(&fixture.request, &RealFs),
            MigrationOutcome::NotNeeded
        );
        assert_eq!(fs::read_link(&fixture.request.path).unwrap(), link);
        assert!(fixture.legacy().join("settings.json").exists());
        if payload == "dangling" {
            fixture.assert_consumers(fixture.legacy());
        }
    }
}

#[test]
fn related_paths_do_not_recurse_or_remove_an_ancestor() {
    for nested_target in [false, true] {
        let mut fixture = Fixture::new();
        fixture.request.path = if nested_target {
            fixture.legacy().join("nested/code")
        } else {
            fixture.base.path().to_path_buf()
        };
        assert!(matches!(
            migrate_requested(&fixture.request, &RealFs),
            MigrationOutcome::NotNeeded | MigrationOutcome::RetainedLegacy
        ));
        assert!(fixture.legacy().join("settings.json").exists());
        assert!(!fixture.legacy().join("nested").exists());
    }
}

#[cfg(unix)]
#[test]
fn parent_symlink_alias_is_checked_before_creating_descendants() {
    let mut fixture = Fixture::new();
    let alias = fixture.base.path().join("alias");
    std::os::unix::fs::symlink(fixture.legacy(), &alias).unwrap();
    fixture.request.path = alias.join("new-parent/code");
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::RetainedLegacy
    );
    assert!(!fixture.legacy().join("new-parent").exists());
    assert!(fixture.legacy().join("settings.json").exists());
}

#[test]
fn marker_never_overwrites_existing_user_entry() {
    let fixture = Fixture::new();
    fs::write(fixture.legacy().join(MARKER), b"original-marker").unwrap();
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::Published
    );
    assert_eq!(
        fs::read(fixture.request.path.join(MARKER)).unwrap(),
        b"original-marker"
    );
}

#[test]
fn typed_resolution_preserves_override_precedence_and_empty_values() {
    let fixture = Fixture::new();
    let home = fixture.base.path();
    let explicit = home.join("custom");
    let managed = home.join("managed");
    let test = home.join("test");
    for (test_home, override_home, coven_home, parent, source, path) in [
        (
            Some(test.to_str().unwrap()),
            Some(explicit.to_str().unwrap()),
            Some(managed.to_str().unwrap()),
            Some("coven"),
            ConfigHomeSource::TestOverride,
            test.join(".coven-code"),
        ),
        (
            None,
            Some(explicit.to_str().unwrap()),
            Some(managed.to_str().unwrap()),
            Some("coven"),
            ConfigHomeSource::ExplicitOverride,
            explicit.clone(),
        ),
        (
            Some(""),
            Some(""),
            Some(managed.to_str().unwrap()),
            None,
            ConfigHomeSource::Managed,
            managed.join("code"),
        ),
        (
            None,
            Some(""),
            Some(""),
            Some("coven"),
            ConfigHomeSource::Managed,
            home.join(".coven/code"),
        ),
        (
            Some(""),
            Some(""),
            Some(""),
            Some(""),
            ConfigHomeSource::Standalone,
            home.join(".coven-code"),
        ),
    ] {
        let request = resolve_config_home(Some(home), test_home, override_home, coven_home, parent);
        assert_eq!(request.source, source);
        assert_eq!(request.path, path);
        if source != ConfigHomeSource::Managed {
            assert_eq!(
                migrate_requested(&request, &RealFs),
                MigrationOutcome::NotNeeded
            );
            assert_eq!(request.effective_path(), path);
            assert!(fixture.legacy().join("settings.json").exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn legacy_root_symlink_is_not_relocated() {
    let fixture = Fixture::new();
    let saved = fixture.base.path().join("saved");
    fs::rename(fixture.legacy(), &saved).unwrap();
    std::os::unix::fs::symlink(&saved, fixture.legacy()).unwrap();
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::NotNeeded
    );
    fixture.assert_consumers(fixture.legacy());
    assert!(!fixture.request.path.exists());
}

#[cfg(unix)]
#[test]
fn safe_parent_alias_keeps_requested_path_usable() {
    let mut fixture = Fixture::new();
    let actual = fixture.base.path().join("actual-managed");
    fs::create_dir(&actual).unwrap();
    let alias = fixture.base.path().join("managed-alias");
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    fixture.request.path = alias.join("code");
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::Published
    );
    fixture.assert_consumers(&fixture.request.path);
    assert_eq!(fs::read_link(&alias).unwrap(), actual);
}

#[cfg(unix)]
#[test]
fn dangling_legacy_root_is_not_selected_as_usable_home() {
    let fixture = Fixture::new();
    fs::remove_dir_all(fixture.legacy()).unwrap();
    std::os::unix::fs::symlink("missing", fixture.legacy()).unwrap();
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::NotNeeded
    );
    assert_eq!(fixture.request.effective_path(), fixture.request.path);
    assert_eq!(
        fs::read_link(fixture.legacy()).unwrap(),
        Path::new("missing")
    );
}

#[test]
fn requested_home_test_scope_restores_outer_request_after_panic() {
    let first = Fixture::new();
    let second = Fixture::new();
    with_requested_home_for_test(first.request.clone(), || {
        assert_eq!(crate::config::config_home(), first.legacy());
        let result = std::panic::catch_unwind(|| {
            with_requested_home_for_test(second.request.clone(), || {
                assert_eq!(crate::config::config_home(), second.legacy());
                panic!("injected consumer panic");
            })
        });
        assert!(result.is_err());
        assert_eq!(crate::config::config_home(), first.legacy());
    });
}

#[test]
fn r1_ordinary_writes_after_deferred_move_remain_readable() {
    let fixture = Fixture::new();
    #[cfg(unix)]
    fixture.links_and_modes();
    let original = snapshot(fixture.legacy());
    let faults = Faults {
        rename_error: Some(io::ErrorKind::CrossesDevices),
        ..Default::default()
    };
    let outcome = migrate_requested(&fixture.request, &faults);
    assert_eq!(outcome, MigrationOutcome::RetainedLegacy);
    assert!(!fixture.request.path.exists());
    assert_eq!(faults.rename_calls.get(), 1);
    assert_eq!(snapshot(fixture.legacy()), original);
    fixture.assert_consumers(fixture.legacy());
    #[cfg(unix)]
    fixture.assert_links_and_modes(fixture.legacy());
    fs::write(
        fixture.legacy().join("settings.json"),
        br#"{"config":{"model":"new-model-after-deferral"}}"#,
    )
    .unwrap();
    let mut session = crate::history::ConversationSession::new("new-model-after-deferral".into());
    session.id = "new-session-after-deferral".into();
    session.add_message(crate::Message::user("new message after deferral"));
    fs::write(
        fixture
            .legacy()
            .join("sessions/new-session-after-deferral.json"),
        serde_json::to_vec(&session).unwrap(),
    )
    .unwrap();
    with_requested_home_for_test(fixture.request.clone(), || {
        assert_eq!(crate::config::config_home(), fixture.legacy());
        assert_eq!(
            Settings::load_sync().unwrap().config.model.as_deref(),
            Some("new-model-after-deferral"),
            "outcome: {outcome:?}"
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let session = runtime
            .block_on(crate::history::load_session("new-session-after-deferral"))
            .unwrap();
        assert_eq!(
            session.messages[0].get_text(),
            Some("new message after deferral")
        );
        assert!(runtime
            .block_on(crate::history::load_session("retained-session"))
            .is_ok());
    });
}

#[test]
fn r1_failed_atomic_move_retains_complete_home_without_snapshots() {
    let fixture = Fixture::new();
    #[cfg(unix)]
    fixture.links_and_modes();
    let original = snapshot(fixture.legacy());
    let faults = Faults {
        rename_error: Some(io::ErrorKind::CrossesDevices),
        ..Default::default()
    };
    for _ in 0..2 {
        faults.rename_calls.set(0);
        assert_eq!(
            migrate_requested(&fixture.request, &faults),
            MigrationOutcome::RetainedLegacy
        );
        assert_eq!(faults.rename_calls.get(), 1);
        assert_eq!(snapshot(fixture.legacy()), original);
        assert!(!fixture.request.path.exists());
        assert_eq!(
            fs::read_dir(fixture.request.path.parent().unwrap())
                .unwrap()
                .count(),
            0
        );
        fixture.assert_consumers(fixture.legacy());
        #[cfg(unix)]
        fixture.assert_links_and_modes(fixture.legacy());
    }
}

#[cfg(unix)]
#[test]
fn r1_replaced_source_cannot_move_an_independent_symlink_target() {
    let fixture = Fixture::new();
    let observed = DirectoryIdentity::open(fixture.legacy()).unwrap();
    let original = snapshot(fixture.legacy());
    let retained = fixture.base.path().join("original-retained");
    fs::rename(fixture.legacy(), &retained).unwrap();
    let independent = fixture.base.path().join("independent");
    fs::create_dir(&independent).unwrap();
    fs::write(independent.join("settings.json"), b"independent-bytes").unwrap();
    std::os::unix::fs::symlink(&independent, fixture.legacy()).unwrap();
    let result = relocate(fixture.legacy(), &fixture.request.path, observed, &RealFs);
    assert_eq!(
        fs::read(independent.join("settings.json")).ok().as_deref(),
        Some(b"independent-bytes".as_slice()),
        "outcome: {result:?}"
    );
    assert!(result.is_err());
    assert_eq!(snapshot(&retained), original);
    assert_eq!(fs::read_link(fixture.legacy()).unwrap(), independent);
    assert!(!fixture.request.path.parent().unwrap().exists());
}

#[cfg(unix)]
#[test]
fn r1_listable_but_unsearchable_target_uses_readable_legacy() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let original = snapshot(fixture.legacy());
    fs::create_dir_all(&fixture.request.path).unwrap();
    fs::write(
        fixture.request.path.join("settings.json"),
        b"unreachable-target-bytes",
    )
    .unwrap();
    fs::set_permissions(&fixture.request.path, fs::Permissions::from_mode(0o400)).unwrap();
    struct RestoreSearch(PathBuf);
    impl Drop for RestoreSearch {
        fn drop(&mut self) {
            fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    let _restore = RestoreSearch(fixture.request.path.clone());
    assert!(fs::read_dir(&fixture.request.path)
        .unwrap()
        .next()
        .is_some());
    if fs::read(fixture.request.path.join("settings.json")).is_ok() {
        // A privileged process can traverse this directory despite the mode;
        // that environment cannot establish this native permission regression.
        eprintln!("native permission case not applicable: process bypasses directory search mode");
        return;
    }
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::NotNeeded
    );
    fixture.assert_consumers(fixture.legacy());
    assert_eq!(
        fs::metadata(&fixture.request.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o400
    );
    drop(_restore);
    assert_eq!(
        fs::read(fixture.request.path.join("settings.json")).unwrap(),
        b"unreachable-target-bytes"
    );
    assert_eq!(snapshot(fixture.legacy()), original);
}

#[test]
fn r1_replaced_source_directory_is_rejected_before_mutation() {
    let fixture = Fixture::new();
    let observed = DirectoryIdentity::open(fixture.legacy()).unwrap();
    let original = snapshot(fixture.legacy());
    let retained = fixture.base.path().join("original-retained");
    fs::rename(fixture.legacy(), &retained).unwrap();
    fs::create_dir(fixture.legacy()).unwrap();
    fs::write(fixture.legacy().join("settings.json"), b"replacement-bytes").unwrap();
    assert!(relocate(fixture.legacy(), &fixture.request.path, observed, &RealFs).is_err());
    assert_eq!(snapshot(&retained), original);
    assert_eq!(
        fs::read(fixture.legacy().join("settings.json")).unwrap(),
        b"replacement-bytes"
    );
    assert!(!fixture.request.path.parent().unwrap().exists());
}

#[cfg(unix)]
#[test]
fn r1_listable_but_unsearchable_legacy_is_not_selected() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let original = snapshot(fixture.legacy());
    fs::set_permissions(fixture.legacy(), fs::Permissions::from_mode(0o400)).unwrap();
    struct RestoreSearch(PathBuf);
    impl Drop for RestoreSearch {
        fn drop(&mut self) {
            fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    let _restore = RestoreSearch(fixture.legacy().to_path_buf());
    if fs::read(fixture.legacy().join("settings.json")).is_ok() {
        eprintln!("native permission case not applicable: process bypasses directory search mode");
        return;
    }
    assert_eq!(fixture.request.effective_path(), fixture.request.path);
    drop(_restore);
    assert_eq!(snapshot(fixture.legacy()), original);
}

#[cfg(unix)]
#[test]
fn r1_dangling_child_links_are_usable_home_data() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.request.path).unwrap();
    std::os::unix::fs::symlink("absent-child", fixture.request.path.join("dangling")).unwrap();
    assert!(has_home_data(&fixture.request.path));
    assert_eq!(fixture.request.effective_path(), fixture.request.path);
    assert_eq!(
        migrate_requested(&fixture.request, &RealFs),
        MigrationOutcome::NotNeeded
    );
    assert_eq!(
        fs::read_link(fixture.request.path.join("dangling")).unwrap(),
        Path::new("absent-child")
    );
    assert!(!fixture.request.path.join("absent-child").exists());
}
