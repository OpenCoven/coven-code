//! #411: an approval covers a file as it stood when the approval was asked for.
//! A file that changes during the wait is left alone and the model is told. Only
//! handlers that opt in get this; every other caller keeps its existing behavior.

use claurst_core::permissions::{PermissionDecision, PermissionHandler, PermissionRequest};
use claurst_core::{CostTracker, PermissionMode};
use claurst_tools::{
    ApplyPatchTool, BatchEditTool, FileEditTool, FileWriteTool, NotebookEditTool, Tool,
    ToolContext, ToolResult,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicUsize, Arc};

const REFUSAL: &str = "changed while waiting for approval";

/// Stands in for someone else touching the file while the panel is open.
enum WhileAsking {
    Nothing,
    Rewrite(String),
    Delete,
}

struct Approves {
    bound: bool,
    target: PathBuf,
    while_asking: WhileAsking,
}

impl PermissionHandler for Approves {
    fn check_permission(&self, _: &PermissionRequest) -> PermissionDecision {
        PermissionDecision::Allow
    }

    fn request_permission(&self, _: &PermissionRequest) -> PermissionDecision {
        match &self.while_asking {
            WhileAsking::Nothing => {}
            WhileAsking::Rewrite(content) => std::fs::write(&self.target, content).unwrap(),
            WhileAsking::Delete => std::fs::remove_file(&self.target).unwrap(),
        }
        PermissionDecision::Allow
    }

    fn binds_approvals_to_content(&self) -> bool {
        self.bound
    }
}

fn context(root: &Path, handler: Approves) -> ToolContext {
    ToolContext {
        working_dir: root.to_path_buf(),
        permission_mode: PermissionMode::Default,
        permission_handler: Arc::new(handler),
        cost_tracker: CostTracker::new(),
        session_id: "approval-binding".into(),
        goal_turn: Default::default(),
        file_history: Arc::new(parking_lot::Mutex::new(
            claurst_core::file_history::FileHistory::new(),
        )),
        current_turn: Arc::new(AtomicUsize::new(0)),
        non_interactive: false,
        mcp_manager: None,
        config: Default::default(),
        managed_agent_config: None,
        completion_notifier: None,
        pending_permissions: None,
        permission_manager: None,
        user_question_tx: None,
    }
}

const NOTEBOOK: &str = r#"{"cells":[{"id":"c1","cell_type":"code","source":"old line","metadata":{},"outputs":[]}],"metadata":{},"nbformat":4,"nbformat_minor":5}"#;

struct Case {
    tool: &'static dyn Tool,
    file: &'static str,
    original: String,
    /// Still satisfies the tool, so only the binding can refuse it.
    changed: String,
    input: Value,
}

fn cases() -> Vec<Case> {
    let text = |tool: &'static dyn Tool, file: &'static str, input: Value| Case {
        tool,
        file,
        original: "old line\n".to_string(),
        changed: "old line\nadded by someone else\n".to_string(),
        input,
    };
    let edit =
        json!({"file_path": "notes.txt", "old_string": "old line", "new_string": "new line"});
    vec![
        text(
            &FileWriteTool,
            "notes.txt",
            json!({"file_path": "notes.txt", "content": "from the model\n"}),
        ),
        text(&FileEditTool, "notes.txt", edit.clone()),
        text(&BatchEditTool, "notes.txt", json!({"edits": [edit]})),
        text(
            &ApplyPatchTool,
            "notes.txt",
            json!({"patch": "--- a/notes.txt\n+++ b/notes.txt\n@@ -1 +1 @@\n-old line\n+new line\n"}),
        ),
        Case {
            tool: &NotebookEditTool,
            file: "book.ipynb",
            original: NOTEBOOK.to_string(),
            changed: NOTEBOOK.replace(
                r#""metadata":{},"nbformat""#,
                r#""metadata":{"by":"someone"},"nbformat""#,
            ),
            input: json!({"notebook_path": "book.ipynb", "cell_id": "c1", "new_source": "new line"}),
        },
    ]
}

fn run(case: &Case, bound: bool, while_asking: WhileAsking) -> (ToolResult, Option<String>) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let target = root.join(case.file);
    std::fs::write(&target, &case.original).unwrap();
    let ctx = context(
        &root,
        Approves {
            bound,
            target: target.clone(),
            while_asking,
        },
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(case.tool.execute(case.input.clone(), &ctx));
    (result, std::fs::read_to_string(&target).ok())
}

#[test]
fn a_file_changed_during_the_wait_is_left_alone_and_the_model_is_told() {
    for case in cases() {
        let name = case.tool.name();
        let (result, on_disk) = run(&case, true, WhileAsking::Rewrite(case.changed.clone()));
        assert!(result.is_error, "{name}: {}", result.content);
        assert!(
            result.content.contains(REFUSAL),
            "{name}: {}",
            result.content
        );
        assert_eq!(on_disk.as_deref(), Some(case.changed.as_str()), "{name}");
    }
}

#[test]
fn a_file_deleted_during_the_wait_is_not_recreated() {
    for case in cases() {
        let name = case.tool.name();
        let (result, on_disk) = run(&case, true, WhileAsking::Delete);
        assert!(result.is_error, "{name}: {}", result.content);
        assert!(
            result.content.contains(REFUSAL),
            "{name}: {}",
            result.content
        );
        assert_eq!(on_disk, None, "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_created_during_the_wait_is_not_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let target = root.join("new.txt");
    let ctx = context(
        &root,
        Approves {
            bound: true,
            target: target.clone(),
            while_asking: WhileAsking::Rewrite("got there first\n".to_string()),
        },
    );
    let result = FileWriteTool
        .execute(
            json!({"file_path": "new.txt", "content": "from the model\n"}),
            &ctx,
        )
        .await;
    assert!(result.content.contains(REFUSAL), "{}", result.content);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "got there first\n"
    );
}

#[test]
fn an_unchanged_file_is_written_as_approved() {
    for case in cases() {
        let name = case.tool.name();
        let (result, on_disk) = run(&case, true, WhileAsking::Nothing);
        assert!(!result.is_error, "{name}: {}", result.content);
        assert_ne!(on_disk.as_deref(), Some(case.original.as_str()), "{name}");
    }
}

#[test]
fn handlers_that_do_not_opt_in_keep_their_existing_behavior() {
    for case in cases() {
        let name = case.tool.name();
        let (result, on_disk) = run(&case, false, WhileAsking::Rewrite(case.changed.clone()));
        assert!(!result.is_error, "{name}: {}", result.content);
        assert_ne!(on_disk.as_deref(), Some(case.changed.as_str()), "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_special_file_is_not_read_before_approval() {
    // Binding only looks at metadata: reading this before asking would never end.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let ctx = context(
        &root,
        Approves {
            bound: true,
            target: root.join("unused"),
            while_asking: WhileAsking::Nothing,
        },
    );
    let reviewed = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ctx.files_under_review([PathBuf::from("/dev/zero")]),
    )
    .await;
    assert!(reviewed.is_ok(), "looking at a device must not read it");
}
