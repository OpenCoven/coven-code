//! Owning a command's whole process tree (#373): on Unix the process group
//! it leads, on Windows a Job Object ([`crate::windows_job`]).

use std::io;
use std::process::{Output, Stdio};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Own the group established at spawn, including while the tool future is
/// dropped. Never discover a group from the caller or from a later PID probe.
#[cfg(unix)]
pub(crate) struct ForegroundGroup {
    pub(crate) group: Option<nix::unistd::Pid>,
}

#[cfg(unix)]
impl ForegroundGroup {
    pub(crate) fn terminate(&mut self) -> Result<(), nix::errno::Errno> {
        match self.group.take() {
            Some(group) => match nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL)
            {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
                Err(error) => Err(error),
            },
            None => Ok(()),
        }
    }
}

#[cfg(unix)]
impl Drop for ForegroundGroup {
    fn drop(&mut self) {
        if let Err(error) = self.terminate() {
            tracing::warn!(%error, "Could not terminate owned foreground process group");
        }
    }
}

/// Run `command` and collect its output, as `Command::output` does, but own
/// everything it starts. If it is still running after `limit`, it is killed
/// with its whole tree and the result is `None`. Dropping the future kills
/// the tree too. An ordinary exit leaves alone what the command started to
/// outlive it, as the foreground shell tools do.
///
/// On Unix the command leads a new process group. A scoped workspace refuses
/// `setpgid`, so there, as for the Bash tool, only the command itself is
/// stopped.
pub(crate) async fn output_within(
    command: &mut Command,
    limit: Duration,
) -> io::Result<Option<Output>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    let owns_group = !crate::workspace_scope::active();
    #[cfg(unix)]
    if owns_group {
        command.process_group(0);
    }
    #[cfg(windows)]
    let (mut child, job) = crate::windows_job::spawn(command).map_err(|error| match error {
        crate::windows_job::SpawnError::NotStarted(error) => error,
        stopped => io::Error::other(stopped.report("Failed to start")),
    })?;
    #[cfg(not(windows))]
    let mut child = command.spawn()?;

    #[cfg(unix)]
    let mut group = ForegroundGroup { group: None };
    #[cfg(unix)]
    if owns_group {
        match child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .filter(|pid| *pid >= 2)
        {
            Some(pid) => group.group = Some(nix::unistd::Pid::from_raw(pid)),
            None => {
                let _ = child.kill().await;
                return Err(io::Error::other("could not establish process ownership"));
            }
        }
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let run = async {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (read_out, read_err) =
            tokio::join!(read_all(stdout, &mut out), read_all(stderr, &mut err));
        read_out?;
        read_err?;
        let status = child.wait().await?;
        Ok::<_, io::Error>(Output {
            status,
            stdout: out,
            stderr: err,
        })
    };
    match tokio::time::timeout(limit, run).await {
        Ok(Ok(output)) => {
            #[cfg(unix)]
            {
                group.group = None;
            }
            #[cfg(windows)]
            if let Some(job) = job {
                job.release();
            }
            Ok(Some(output))
        }
        // Dropping the group or the job kills what is left.
        Ok(Err(error)) => Err(error),
        Err(_) => {
            #[cfg(unix)]
            if let Err(error) = group.terminate() {
                tracing::warn!(%error, "Could not terminate a timed-out command's process group");
            }
            #[cfg(windows)]
            if let Some(job) = &job {
                job.terminate();
            }
            let _ = child.kill().await;
            Ok(None)
        }
    }
}

async fn read_all(pipe: Option<impl AsyncRead + Unpin>, buffer: &mut Vec<u8>) -> io::Result<()> {
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(buffer).await?;
    }
    Ok(())
}
