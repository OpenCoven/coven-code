//! Shared line draining for piped shell subprocesses.
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

pub(crate) async fn drain_lines<R: AsyncRead + Unpin>(
    pipe: Option<R>,
    mut on_line: impl FnMut(String),
) {
    if let Some(pipe) = pipe {
        let mut lines = BufReader::new(pipe).lines();
        // Preserve the shell tools' existing EOF and read-error behavior.
        while let Ok(Some(line)) = lines.next_line().await {
            on_line(line);
        }
    }
}

/// Drain both background pipes while retaining stdout-first registry output.
pub(crate) async fn drain_background(
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    task_id: &str,
) {
    use claurst_core::tasks::global_registry;

    // The callbacks share state, but never hold this lock across an await.
    let pending = parking_lot::Mutex::new((false, Vec::<String>::new()));
    tokio::join!(
        async {
            drain_lines(stdout, |line| {
                global_registry().append_output(task_id, &line);
            })
            .await;
            let mut state = pending.lock();
            for line in state.1.drain(..) {
                global_registry().append_output(task_id, &format!("STDERR: {}", line));
            }
            state.0 = true;
        },
        drain_lines(stderr, |line| {
            let mut state = pending.lock();
            if state.0 {
                global_registry().append_output(task_id, &format!("STDERR: {}", line));
            } else {
                state.1.push(line);
            }
        }),
    );
}
