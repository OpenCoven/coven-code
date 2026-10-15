use claurst_core::{
    permissions::{PermissionAction, PermissionRule, PermissionScope},
    CostTracker, ManagedInteractivePermissionHandler, PermissionManager, PermissionMode, Settings,
};
use claurst_tools::{FileEditTool, FileWriteTool, Tool, ToolContext};
use serde_json::json;
use std::{
    path::Path,
    sync::{atomic::AtomicUsize, Arc, Mutex},
};

fn context(root: &Path, mode: PermissionMode, rules: Vec<PermissionRule>) -> ToolContext {
    let mut manager = PermissionManager::new(mode.clone(), &Settings::default());
    for rule in rules {
        manager.add_rule(rule);
    }
    let manager = Arc::new(Mutex::new(manager));
    ToolContext {
        working_dir: root.to_path_buf(),
        permission_mode: mode,
        permission_handler: Arc::new(ManagedInteractivePermissionHandler::new(Arc::clone(
            &manager,
        ))),
        cost_tracker: CostTracker::new(),
        session_id: "write-path-fixture".into(),
        goal_turn: Default::default(),
        file_history: Arc::new(parking_lot::Mutex::new(
            claurst_core::file_history::FileHistory::new(),
        )),
        current_turn: Arc::new(AtomicUsize::new(0)),
        non_interactive: true,
        mcp_manager: None,
        config: Default::default(),
        managed_agent_config: None,
        completion_notifier: None,
        pending_permissions: None,
        permission_manager: Some(manager),
        user_question_tx: None,
    }
}

fn rule(tool: &str, path: Option<&Path>, action: PermissionAction) -> PermissionRule {
    PermissionRule {
        tool_name: Some(tool.into()),
        path_pattern: path.map(|p| glob::Pattern::escape(p.to_str().unwrap())),
        action,
        scope: PermissionScope::Session,
    }
}

#[tokio::test]
async fn path_deny_overrides_broad_allow_and_accept_edits() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let target = root.join("protected.txt");
    for mode in [PermissionMode::Default, PermissionMode::AcceptEdits] {
        for tool in [&FileWriteTool as &dyn Tool, &FileEditTool as &dyn Tool] {
            std::fs::write(&target, "original").unwrap();
            let ctx = context(
                &root,
                mode.clone(),
                vec![
                    rule(tool.name(), None, PermissionAction::Allow),
                    rule(tool.name(), Some(&target), PermissionAction::Deny),
                ],
            );
            let result = tool.execute(json!({"file_path": "protected.txt", "content": "changed", "old_string": "original", "new_string": "changed"}), &ctx).await;
            assert!(
                result.is_error,
                "{} ignored path deny in {mode:?}",
                tool.name()
            );
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        }
    }
}

#[tokio::test]
async fn new_target_denial_creates_no_parent_and_scoped_allow_works() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let target = root.join("new/nested/file.txt");
    let ctx = context(
        &root,
        PermissionMode::Default,
        vec![
            rule("Write", None, PermissionAction::Allow),
            rule("Write", Some(&target), PermissionAction::Deny),
        ],
    );
    let result = FileWriteTool
        .execute(
            json!({"file_path": "new/nested/file.txt", "content": "new"}),
            &ctx,
        )
        .await;
    assert!(result.is_error);
    assert!(!root.join("new").exists());
    let ctx = context(
        &root,
        PermissionMode::Default,
        vec![rule("Write", Some(&target), PermissionAction::Allow)],
    );
    let result = FileWriteTool
        .execute(
            json!({"file_path": "new/nested/file.txt", "content": "new"}),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(std::fs::read_to_string(target).unwrap(), "new");
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_parent_and_leaf_cannot_evade_target_deny() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_root = outside.path().canonicalize().unwrap();
    std::os::unix::fs::symlink(&outside_root, root.join("alias")).unwrap();
    let target = outside_root.join("protected.txt");
    std::fs::write(&target, "original").unwrap();
    std::os::unix::fs::symlink(&target, root.join("leaf.txt")).unwrap();
    for path in ["alias/protected.txt", "leaf.txt"] {
        for tool in [&FileWriteTool as &dyn Tool, &FileEditTool as &dyn Tool] {
            let ctx = context(
                &root,
                PermissionMode::AcceptEdits,
                vec![
                    rule(tool.name(), None, PermissionAction::Allow),
                    rule(tool.name(), Some(&target), PermissionAction::Deny),
                ],
            );
            let result = tool.execute(json!({"file_path": path, "content": "changed", "old_string": "original", "new_string": "changed"}), &ctx).await;
            assert!(
                result.is_error,
                "{} ignored resolved target {path}",
                tool.name()
            );
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        }
    }
}

#[tokio::test]
async fn edit_scoped_allow_and_accept_edits_preserve_normal_behavior() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let target = root.join("editable.txt");
    for (mode, rules, allowed) in [
        (PermissionMode::Default, vec![], false),
        (
            PermissionMode::Default,
            vec![rule("Edit", Some(&target), PermissionAction::Allow)],
            true,
        ),
        (PermissionMode::AcceptEdits, vec![], true),
        (PermissionMode::Plan, vec![], false),
    ] {
        std::fs::write(&target, "original").unwrap();
        let ctx = context(&root, mode, rules);
        let result = FileEditTool.execute(json!({"file_path": "editable.txt", "old_string": "original", "new_string": "changed"}), &ctx).await;
        assert_eq!(result.is_error, !allowed);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            if allowed { "changed" } else { "original" }
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn new_target_through_symlink_parent_is_denied_without_side_effects() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_root = outside.path().canonicalize().unwrap();
    std::os::unix::fs::symlink(&outside_root, root.join("alias")).unwrap();
    let target = outside_root.join("nested/new.txt");
    let ctx = context(
        &root,
        PermissionMode::Default,
        vec![
            rule("Write", None, PermissionAction::Allow),
            rule("Write", Some(&target), PermissionAction::Deny),
        ],
    );
    let result = FileWriteTool
        .execute(
            json!({"file_path": "alias/nested/new.txt", "content": "changed"}),
            &ctx,
        )
        .await;
    assert!(result.is_error);
    assert!(!outside_root.join("nested").exists());
}

fn patch_for(path: &str, old: &str, new: &str) -> String {
    format!("--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-{old}\n+{new}\n")
}

#[tokio::test]
async fn patch_denied_later_target_preserves_all_files_including_dry_run() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    let patch = patch_for("first.txt", "old", "new") + &patch_for("second.txt", "old", "new");
    for dry_run in [false, true] {
        std::fs::write(&first, "old\n").unwrap();
        std::fs::write(&second, "old\n").unwrap();
        let ctx = context(
            &root,
            PermissionMode::Default,
            vec![
                rule("ApplyPatch", None, PermissionAction::Allow),
                rule("ApplyPatch", Some(&second), PermissionAction::Deny),
            ],
        );
        let result = claurst_tools::ApplyPatchTool
            .execute(json!({"patch": patch, "dry_run": dry_run}), &ctx)
            .await;
        assert!(
            result.is_error && result.content.contains("Permission denied"),
            "{result:?}"
        );
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "old\n");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "old\n");
    }
}

#[tokio::test]
async fn patch_checks_all_permissions_before_reading_content() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    // Reading the first target as UTF-8 would fail before reaching the deny.
    std::fs::write(root.join("first.txt"), [0xff]).unwrap();
    let second = root.join("second.txt");
    std::fs::write(&second, "old\n").unwrap();
    let patch = patch_for("first.txt", "old", "new") + &patch_for("second.txt", "old", "new");
    for dry_run in [false, true] {
        let ctx = context(
            &root,
            PermissionMode::Default,
            vec![
                rule("ApplyPatch", None, PermissionAction::Allow),
                rule("ApplyPatch", Some(&second), PermissionAction::Deny),
            ],
        );
        let result = claurst_tools::ApplyPatchTool
            .execute(json!({"patch": patch, "dry_run": dry_run}), &ctx)
            .await;
        assert!(
            result.content.contains("Permission denied"),
            "read occurred before all approvals: {result:?}"
        );
    }
}

#[tokio::test]
async fn patch_scoped_allow_preserves_apply_and_dry_run() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let target = root.join("allowed.txt");
    for dry_run in [true, false] {
        std::fs::write(&target, "old\n").unwrap();
        let ctx = context(
            &root,
            PermissionMode::Default,
            vec![rule("ApplyPatch", Some(&target), PermissionAction::Allow)],
        );
        let result = claurst_tools::ApplyPatchTool
            .execute(
                json!({"patch": patch_for("allowed.txt", "old", "new"), "dry_run": dry_run}),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{result:?}");
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            if dry_run { "old\n" } else { "new\n" }
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn patch_symlink_target_deny_applies_to_validation_and_mutation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let target = root.join("protected.txt");
    std::fs::write(&target, "old\n").unwrap();
    std::os::unix::fs::symlink(&target, root.join("alias.txt")).unwrap();
    for dry_run in [false, true] {
        let ctx = context(
            &root,
            PermissionMode::Default,
            vec![
                rule("ApplyPatch", None, PermissionAction::Allow),
                rule("ApplyPatch", Some(&target), PermissionAction::Deny),
            ],
        );
        let result = claurst_tools::ApplyPatchTool
            .execute(
                json!({"patch": patch_for("alias.txt", "old", "new"), "dry_run": dry_run}),
                &ctx,
            )
            .await;
        assert!(result.is_error, "{result:?}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old\n");
    }
}

#[tokio::test]
async fn patch_late_validation_failure_preserves_earlier_target() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("first.txt"), "old\n").unwrap();
    std::fs::write(root.join("second.txt"), "different\n").unwrap();
    let ctx = context(
        &root,
        PermissionMode::Default,
        vec![rule("ApplyPatch", None, PermissionAction::Allow)],
    );
    let patch = patch_for("first.txt", "old", "new") + &patch_for("second.txt", "old", "new");
    let result = claurst_tools::ApplyPatchTool
        .execute(json!({"patch": patch}), &ctx)
        .await;
    assert!(
        result.is_error && result.content.contains("Context not found"),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("first.txt")).unwrap(),
        "old\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("second.txt")).unwrap(),
        "different\n"
    );
}

#[tokio::test]
async fn patch_dry_run_requires_approval_in_default_and_plan_modes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let target = root.join("target.txt");
    std::fs::write(&target, "old\n").unwrap();
    for mode in [PermissionMode::Default, PermissionMode::Plan] {
        let ctx = context(&root, mode, vec![]);
        let result = claurst_tools::ApplyPatchTool
            .execute(
                json!({"patch": patch_for("target.txt", "old", "new"), "dry_run": true}),
                &ctx,
            )
            .await;
        assert!(
            result.is_error && result.content.contains("Permission denied"),
            "{result:?}"
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old\n");
    }
}
