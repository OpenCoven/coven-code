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

    let task_id = global_registry().register(task);

    let task_id_clone = task_id.clone();
    let command_clone = command.to_owned();

    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        let child = if cfg!(windows) {
            Command::new("cmd")
                .arg("/C")
                .arg(&command_clone)
                .current_dir(&cwd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .spawn()
        } else {
            Command::new("bash")
                .arg("-c")
                .arg(&command_clone)
                .current_dir(&cwd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .spawn()
        };

        match child {
            Ok(mut c) => {
                let result = tokio::time::timeout_at(deadline, async {
                    // Record PID in the registry.
                    if let Some(pid) = c.id() {
                        global_registry().set_pid(&task_id_clone, pid);
                    }

                    let stdout = c.stdout.take();
                    let stderr = c.stderr.take();
                    crate::shell_pipe::drain_background(stdout, stderr, &task_id_clone).await;

                    match c.wait().await {
                        Ok(status) if status.success() => {
                            global_registry().complete(&task_id_clone);
                        }
                        Ok(status) => {
                            let code = status.code().unwrap_or(-1);
                            global_registry().finish_if_running(
                                &task_id_clone,
                                claurst_core::tasks::TaskStatus::Failed(format!(
                                    "exit code {}",
                                    code
                                )),
                            );
                        }
                        Err(e) => {
                            global_registry().finish_if_running(
                                &task_id_clone,
                                claurst_core::tasks::TaskStatus::Failed(e.to_string()),
                            );
                        }
                    }
                })
                .await;
                if result.is_err() {
                    let mut message = format!("timed out after {}ms", timeout_ms);
                    // kill() also waits for the owned child to be reaped.
                    if let Err(error) = c.kill().await {
                        message.push_str(&format!("; failed to kill/reap child: {}", error));
                    }
                    global_registry().finish_if_running(
                        &task_id_clone,
                        claurst_core::tasks::TaskStatus::Failed(message),
                    );
                }
            }
            Err(e) => {
                global_registry().finish_if_running(
                    &task_id_clone,
                    claurst_core::tasks::TaskStatus::Failed(e.to_string()),
                );
            }
        }
    });

    task_id
}
