use claurst_tools::workspace_scope::WorkspaceScope;
use std::{fs, io, path::Path};

fn assert_diagnostic(
    error: io::Error,
    kind: io::ErrorKind,
    operation: &str,
    path: &Path,
    cause: &str,
) {
    assert_eq!(error.kind(), kind);
    let message = error.to_string();
    for part in [operation, &format!("{path:?}"), cause] {
        assert!(message.contains(part), "missing {part:?} in {message:?}");
    }
}

#[test]
fn policy_input_failures_identify_the_operation_and_keep_the_cause() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().canonicalize().unwrap();
    let file = work.join("policy.json");
    let missing = file.symlink_metadata().unwrap_err();
    assert_diagnostic(
        WorkspaceScope::read(&file, &work).unwrap_err(),
        missing.kind(),
        "inspect workspace policy",
        &file,
        &missing.to_string(),
    );

    fs::write(&file, "{").unwrap();
    assert_diagnostic(
        WorkspaceScope::read(&file, &work).unwrap_err(),
        io::ErrorKind::Other,
        "parse workspace policy",
        &file,
        "line 1 column 1",
    );

    let cwd = work.join("missing-work");
    fs::write(
        &file,
        serde_json::json!({
            "version":1, "runId":"admission", "access":"workspace-write",
            "primaryRoot":cwd, "writableRoots":[cwd], "readOnlyRoots":[]
        })
        .to_string(),
    )
    .unwrap();
    let missing = cwd.canonicalize().unwrap_err();
    assert_diagnostic(
        WorkspaceScope::read(&file, &cwd).unwrap_err(),
        missing.kind(),
        "resolve working folder",
        &cwd,
        &missing.to_string(),
    );

    let project = work.join("missing-project");
    fs::write(
        &file,
        serde_json::json!({
            "version": 1, "runId": "admission", "access": "workspace-write",
            "primaryRoot": work, "writableRoots": [work, project], "readOnlyRoots": []
        })
        .to_string(),
    )
    .unwrap();
    assert_diagnostic(
        WorkspaceScope::read(&file, &work).unwrap_err(),
        io::ErrorKind::InvalidInput,
        "inspect workspace folder",
        &project,
        &project.metadata().unwrap_err().to_string(),
    );
}

#[cfg(unix)]
#[test]
fn unreadable_policy_names_the_file_and_keeps_permission_denied() {
    use std::os::unix::fs::PermissionsExt;
    if nix::unistd::geteuid().is_root() {
        return; // Root can read mode-000 files.
    }
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("unreadable-policy");
    fs::write(&file, "{}").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    let denied = fs::File::open(&file).unwrap_err();
    assert_diagnostic(
        WorkspaceScope::read(&file, temp.path()).unwrap_err(),
        io::ErrorKind::PermissionDenied,
        "open workspace policy",
        &file,
        &denied.to_string(),
    );
}
