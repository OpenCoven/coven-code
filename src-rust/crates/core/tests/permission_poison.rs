use claurst_core::{
    ManagedInteractivePermissionHandler, PermissionDecision, PermissionHandler, PermissionManager,
    PermissionMode, PermissionRequest, Settings,
};
use std::sync::{Arc, Mutex};

fn bash_request() -> PermissionRequest {
    PermissionRequest {
        tool_name: "Bash".into(),
        description: "permission fixture; no command executes".into(),
        details: None,
        is_read_only: false,
        path: None,
        working_dir: None,
        allowed_roots: Vec::new(),
        context_description: None,
    }
}

#[test]
fn poisoned_interactive_manager_denies_both_entrypoints() {
    let manager = Arc::new(Mutex::new(PermissionManager::new(
        PermissionMode::Default,
        &Settings::default(),
    )));
    let worker_manager = Arc::clone(&manager);
    let worker = std::thread::spawn(move || {
        let _guard = worker_manager.lock().unwrap();
        panic!("intentional fixture panic to poison the manager");
    });
    assert!(worker.join().is_err());
    assert!(manager.is_poisoned());
    let handler = ManagedInteractivePermissionHandler::new(manager);
    let request = bash_request();
    // Evaluate both methods before asserting, so the failure records both paths.
    let decisions = (
        handler.check_permission(&request),
        handler.request_permission(&request),
    );
    assert_eq!(
        decisions,
        (PermissionDecision::Deny, PermissionDecision::Deny)
    );
}

#[test]
fn healthy_interactive_manager_preserves_prompt_and_allow() {
    let manager = Arc::new(Mutex::new(PermissionManager::new(
        PermissionMode::Default,
        &Settings::default(),
    )));
    let handler = ManagedInteractivePermissionHandler::new(Arc::clone(&manager));
    let request = bash_request();
    assert!(matches!(
        handler.check_permission(&request),
        PermissionDecision::Ask { .. }
    ));
    assert!(matches!(
        handler.request_permission(&request),
        PermissionDecision::Ask { .. }
    ));
    manager.lock().unwrap().add_session_allow("Bash");
    assert_eq!(
        handler.check_permission(&request),
        PermissionDecision::Allow
    );
    assert_eq!(
        handler.request_permission(&request),
        PermissionDecision::Allow
    );
}
