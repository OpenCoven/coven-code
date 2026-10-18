use claurst_core::tasks::{BackgroundTask, TaskRegistry, TaskStatus};
use tokio_util::sync::CancellationToken;

#[test]
fn owned_task_cancellation_reaches_runtime_only_when_it_wins() {
    let registry = TaskRegistry::new();
    let cancelled_token = CancellationToken::new();
    let cancelled_id = registry.register(
        BackgroundTask::new("cancellable agent").with_cancel_token(cancelled_token.clone()),
    );
    assert!(registry.finish_if_running(&cancelled_id, TaskStatus::Cancelled));
    assert!(cancelled_token.is_cancelled());

    let completed_token = CancellationToken::new();
    let completed_id = registry.register(
        BackgroundTask::new("completed agent").with_cancel_token(completed_token.clone()),
    );
    assert!(registry.finish_if_running(&completed_id, TaskStatus::Completed));
    assert!(!registry.finish_if_running(&completed_id, TaskStatus::Cancelled));
    assert!(!completed_token.is_cancelled());
}

#[test]
fn completed_and_cancelled_tasks_keep_their_terminal_state() {
    for original in [
        TaskStatus::Completed,
        TaskStatus::Cancelled,
        TaskStatus::Failed("original failure".into()),
    ] {
        for cancel in [false, true] {
            let registry = TaskRegistry::new();
            let id = registry.register(BackgroundTask::new("owned registry fixture"));
            registry.set_pid(&id, 123);
            registry.append_output(&id, "retained output");
            registry.update_status(&id, original.clone());
            let before = registry.get(&id).unwrap();
            assert!(before.completed_at.is_some());
            if cancel {
                registry.cancel(&id);
            } else {
                registry.complete(&id);
            }
            let after = registry.get(&id).unwrap();
            assert_eq!(after.status, original);
            assert_eq!(after.completed_at, before.completed_at);
            assert_eq!(after.pid, before.pid);
            assert_eq!(after.output, before.output);
            assert_eq!(after.name, before.name);
            assert_eq!(after.started_at, before.started_at);
        }
    }
}

#[test]
fn terminal_wrappers_finish_running_and_ignore_unknown_tasks() {
    let registry = TaskRegistry::new();
    registry.complete("unknown");
    registry.cancel("unknown");
    assert!(registry.list().is_empty());
    for cancel in [false, true] {
        let id = registry.register(BackgroundTask::new("running"));
        if cancel {
            registry.cancel(&id);
        } else {
            registry.complete(&id);
        }
        let task = registry.get(&id).unwrap();
        assert_eq!(
            task.status,
            if cancel {
                TaskStatus::Cancelled
            } else {
                TaskStatus::Completed
            }
        );
        assert!(task.completed_at.is_some());
    }
}

#[test]
fn guarded_finish_rejects_running_and_preserves_generic_updates() {
    let registry = TaskRegistry::new();
    assert!(!registry.finish_if_running("unknown", TaskStatus::Completed));
    let id = registry.register(BackgroundTask::new("generic reset control"));
    registry.set_pid(&id, 123);
    registry.append_output(&id, "retained output");
    assert!(!registry.finish_if_running(&id, TaskStatus::Running));
    assert!(registry.get(&id).unwrap().completed_at.is_none());
    assert!(registry.finish_if_running(&id, TaskStatus::Failed("first".into())));
    let completed_at = registry.get(&id).unwrap().completed_at;
    assert!(!registry.finish_if_running(&id, TaskStatus::Cancelled));
    registry.update_status(&id, TaskStatus::Completed);
    assert_eq!(registry.get(&id).unwrap().status, TaskStatus::Completed);
    registry.update_status(&id, TaskStatus::Running);
    assert_eq!(registry.get(&id).unwrap().completed_at, completed_at);
    assert!(registry.finish_if_running(&id, TaskStatus::Cancelled));
    let final_task = registry.get(&id).unwrap();
    assert_eq!(final_task.status, TaskStatus::Cancelled);
    assert_eq!(final_task.completed_at, completed_at);
    assert_eq!(final_task.pid, Some(123));
    assert_eq!(final_task.output, ["retained output"]);
}

#[test]
fn competing_terminal_updates_have_one_winner() {
    let registry = TaskRegistry::new();
    let id = registry.register(BackgroundTask::new("competing terminal updates"));
    let barrier = std::sync::Barrier::new(3);
    let (completed, cancelled) = std::thread::scope(|scope| {
        let complete = scope.spawn(|| {
            barrier.wait();
            registry.finish_if_running(&id, TaskStatus::Completed)
        });
        let cancel = scope.spawn(|| {
            barrier.wait();
            registry.finish_if_running(&id, TaskStatus::Cancelled)
        });
        barrier.wait();
        (complete.join().unwrap(), cancel.join().unwrap())
    });
    assert_ne!(completed, cancelled);
    let winner = registry.get(&id).unwrap();
    assert_eq!(
        winner.status,
        if completed {
            TaskStatus::Completed
        } else {
            TaskStatus::Cancelled
        }
    );
    assert!(winner.completed_at.is_some());
    assert!(!registry.finish_if_running(&id, TaskStatus::Failed("late failure".into())));
    assert_eq!(registry.get(&id).unwrap().completed_at, winner.completed_at);
}
