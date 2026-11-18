//! Shared background shell task registration and execution.
use claurst_core::tasks::{global_registry, BackgroundTask};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

pub(crate) fn spawn(command: &str, cwd: PathBuf, timeout_ms: u64) -> String {
    let mut name_end = command.len().min(60);
    while !command.is_char_boundary(name_end) {
        name_end -= 1;
    }
    let task_name = format!("bg: {}", &command[..name_end]);
    let mut task = BackgroundTask::new(&task_name);
    task.pid = None; // Will be set after spawn
                     // On Windows, cancelling the task, as the monitor tool does, cancels this
                     // token, and the task then kills the command's job (#373).
    #[cfg(windows)]
    let stop = tokio_util::sync::CancellationToken::new();
    #[cfg(windows)]
    let task = task.with_cancel_token(stop.clone());

    let task_id = global_registry().register(task);

    let task_id_clone = task_id.clone();
    let command_clone = command.to_owned();

    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        let (shell, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("bash", "-c")
        };
        let (program, args) =
            match crate::workspace_scope::shell(shell, &[flag.into(), command_clone]) {
                Ok(parts) => parts,
                Err(e) => return fail(&task_id_clone, e.to_string()),
            };
        let mut shell = Command::new(program);
        shell
            .args(args)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);
        // The job owns cmd.exe and everything it starts. A timeout or a cancel
        // terminates it, and so does coven-code exiting, which closes its only
        // handle (#373).
        #[cfg(windows)]
        let (mut c, job) = match crate::windows_job::spawn(&mut shell) {
            Ok(spawned) => spawned,
            Err(crate::windows_job::SpawnError::NotStarted(e)) => {
                return fail(&task_id_clone, e.to_string())
            }
            Err(stopped) => return fail(&task_id_clone, stopped.report("Failed to spawn command")),
        };
        #[cfg(not(windows))]
        let mut c = match shell.spawn() {
            Ok(c) => c,
            Err(e) => return fail(&task_id_clone, e.to_string()),
        };

        #[cfg(windows)]
        let stopped = stop.cancelled();
        #[cfg(not(windows))]
        let stopped = std::future::pending::<()>();
        let ended = tokio::select! {
            ended = tokio::time::timeout_at(deadline, async {
                // Record PID in the registry.
                if let Some(pid) = c.id() {
                    global_registry().set_pid(&task_id_clone, pid);
                }

                let stdout = c.stdout.take();
                let stderr = c.stderr.take();
                crate::shell_pipe::drain_background(stdout, stderr, &task_id_clone).await;

                c.wait().await
            }) => Some(ended),
            () = stopped => None,
        };

        match ended {
            Some(Ok(status)) => {
                // An ordinary exit leaves alone what the command started to
                // outlive it.
                #[cfg(windows)]
                if let (Ok(_), Some(job)) = (&status, job) {
                    job.release();
                }
                match status {
                    Ok(status) if status.success() => {
                        global_registry().complete(&task_id_clone);
                    }
                    Ok(status) => {
                        let code = status.code().unwrap_or(-1);
                        fail(&task_id_clone, format!("exit code {}", code));
                    }
                    Err(e) => fail(&task_id_clone, e.to_string()),
                }
            }
            Some(Err(_)) => {
                #[cfg(windows)]
                if let Some(job) = &job {
                    job.terminate();
                }
                let mut message = format!("timed out after {}ms", timeout_ms);
                // kill() also waits for the owned child to be reaped.
                if let Err(error) = c.kill().await {
                    message.push_str(&format!("; failed to kill/reap child: {}", error));
                }
                fail(&task_id_clone, message);
            }
            // Cancelled: the registry already says so.
            None => {
                #[cfg(windows)]
                if let Some(job) = &job {
                    job.terminate();
                }
                if let Err(error) = c.kill().await {
                    tracing::warn!(%error, "Could not kill or reap a cancelled background command");
                }
            }
        }
    });

    task_id
}

/// Record that the task failed, unless it already ended.
fn fail(task_id: &str, reason: String) {
    global_registry().finish_if_running(task_id, claurst_core::tasks::TaskStatus::Failed(reason));
}
