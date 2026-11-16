//! A Job Object for each foreground Windows shell command (#373).
//!
//! Killing `cmd.exe` or `powershell.exe` does not kill what it started. So the
//! shell is created suspended, joins a new job, and only then runs: every
//! process the shell or its descendants create is created inside the job.
//! Terminating the job, or closing its only handle, kills all of them.
//!
//! Only the shell's own process tree is in the job. Work the command hands to
//! a process outside it is not: an elevated process (`Start-Process -Verb
//! RunAs`), a scheduled task, a process created through WMI, a service, or a
//! Linux process under WSL.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use tokio::process::{Child, Command};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    GetProcessIdOfThread, OpenThread, ResumeThread, CREATE_SUSPENDED,
    THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
};

/// The job that owns one command's process tree. Dropping it closes the only
/// handle to the job, and `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` then kills every
/// process still in it, so a cancelled call cleans up as well.
pub(crate) struct Job(OwnedHandle);

impl Job {
    fn new() -> io::Result<Self> {
        // SAFETY: null attributes and name are allowed. They make an unnamed
        // job whose handle no child process inherits.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `handle` is a new, open handle that nothing else owns.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        job.limit(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)?;
        Ok(job)
    }

    /// Replace the job's limits with `flags`.
    fn limit(&self, flags: JOB_OBJECT_LIMIT) -> io::Result<()> {
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: the pointer and size describe `limits`, the structure this
        // information class takes, and it outlives the call.
        let set = unsafe {
            SetInformationJobObject(
                self.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Kill the shell and everything it started, for a timeout.
    pub(crate) fn terminate(&self) {
        // SAFETY: `self.0` is an open job handle.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            // Dropping the job still kills its processes.
            let error = io::Error::last_os_error();
            tracing::warn!(%error, "Could not terminate a foreground command's job");
        }
    }

    /// Close the job without killing what the command left running when it
    /// exited, such as a process it started to outlive it. Unix keeps those
    /// after an ordinary exit as well.
    pub(crate) fn release(self) {
        if let Err(error) = self.limit(0) {
            tracing::warn!(
                %error,
                "Could not release a foreground command's job; closing it kills what the command left running"
            );
        }
    }
}

/// How far resuming the suspended shell got.
enum Resumed {
    /// Every thread of the shell runs.
    All,
    /// No thread of the shell has run, so the command has not started.
    NotStarted(io::Error),
    /// A thread of the shell runs, so the command may have started.
    Partly(io::Error),
}

/// Spawn `command` with its whole process tree in a new job.
///
/// The shell is created suspended and resumed only once it is in the job, so
/// it cannot start anything outside it. This sets `command`'s creation flags,
/// replacing any it had: a caller that needs others, such as
/// `CREATE_NO_WINDOW`, has to pass them in here first.
///
/// If no job can be created or joined, for example under a parent job that
/// does not allow nesting, the command still runs, and cancel and timeout stop
/// only the shell, as before. It is an error only if the shell cannot start,
/// or if it started and could not be fully resumed. Then the shell is stopped,
/// never started a second time.
pub(crate) fn spawn(command: &mut Command) -> io::Result<(Child, Option<Job>)> {
    let job = match Job::new() {
        Ok(job) => job,
        Err(error) => {
            tracing::warn!(%error, "No job for a foreground command; only its shell will be stopped");
            return Ok((command.spawn()?, None));
        }
    };
    let mut child = command.creation_flags(CREATE_SUSPENDED).spawn()?;
    let (Some(process), Some(id)) = (child.raw_handle(), child.id()) else {
        // Only a child that has been waited on has neither.
        return Err(io::Error::other("the new shell has no process handle"));
    };
    // SAFETY: both handles are open: `job` owns one and `child` the other.
    let job = if unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), process) } != 0 {
        Some(job)
    } else {
        let error = io::Error::last_os_error();
        tracing::warn!(%error, "Could not put a foreground command in a job; only its shell will be stopped");
        None
    };
    let resumed = match threads_of(id) {
        Ok(threads) => resume_all(threads, |thread| resume_thread(id, thread)),
        Err(error) => Resumed::NotStarted(error),
    };
    match resumed {
        Resumed::All => Ok((child, job)),
        Resumed::NotStarted(error) => {
            // No thread of the shell has run, so starting it again cannot run
            // the command twice.
            tracing::warn!(%error, "Could not resume a foreground command's shell; starting it again without a job");
            let _ = child.start_kill();
            Ok((command.creation_flags(0).spawn()?, None))
        }
        Resumed::Partly(error) => {
            // The command may have started, so stop it rather than run it again.
            if let Some(job) = &job {
                job.terminate();
            }
            let _ = child.start_kill();
            Err(io::Error::new(
                error.kind(),
                format!("the shell started but could not be fully resumed: {error}"),
            ))
        }
    }
}

/// The IDs of the threads of `process_id`. tokio's `Child` exposes neither
/// std's `Child` nor the suspended shell's thread handle, so take a snapshot.
fn threads_of(process_id: u32) -> io::Result<Vec<u32>> {
    // SAFETY: takes no pointers. A thread snapshot lists every thread in the
    // system, whatever process ID is passed.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `snapshot` is a new, open handle that nothing else owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..THREADENTRY32::default()
    };
    let mut threads = Vec::new();
    // SAFETY: `entry` is a THREADENTRY32 with `dwSize` set, as the walk needs.
    let mut listed = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
    while listed {
        if entry.th32OwnerProcessID == process_id {
            threads.push(entry.th32ThreadID);
        }
        // SAFETY: as for Thread32First.
        listed = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
    }
    Ok(threads)
}

/// Resume each of `threads` with `resume_one`, stopping at the first failure.
/// `resume_one` returns false for an ID that no longer names a thread of the
/// shell. Whether any thread was resumed tells whether the command may have
/// started.
fn resume_all(threads: Vec<u32>, mut resume_one: impl FnMut(u32) -> io::Result<bool>) -> Resumed {
    let mut resumed = false;
    for thread in threads {
        match resume_one(thread) {
            Ok(true) => resumed = true,
            Ok(false) => {}
            Err(error) if resumed => return Resumed::Partly(error),
            Err(error) => return Resumed::NotStarted(error),
        }
    }
    if resumed {
        Resumed::All
    } else {
        Resumed::NotStarted(io::Error::new(
            io::ErrorKind::NotFound,
            "the suspended shell has no thread to resume",
        ))
    }
}

/// Resume thread `thread_id` if it still belongs to `process_id`; a thread ID
/// from the snapshot may since have been reused by another process.
fn resume_thread(process_id: u32, thread_id: u32) -> io::Result<bool> {
    // SAFETY: takes no pointers.
    let thread = unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
            0,
            thread_id,
        )
    };
    if thread.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `thread` is a new, open handle that nothing else owns.
    let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
    // SAFETY: `thread` was opened with THREAD_QUERY_LIMITED_INFORMATION.
    match unsafe { GetProcessIdOfThread(thread.as_raw_handle()) } {
        0 => return Err(io::Error::last_os_error()),
        owner if owner != process_id => return Ok(false),
        _ => {}
    }
    // SAFETY: `thread` was opened with THREAD_SUSPEND_RESUME.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{resume_all, Resumed};
    use std::io;

    fn refused() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    #[test]
    fn a_shell_whose_threads_all_resumed_runs() {
        assert!(matches!(resume_all(vec![7, 8], |_| Ok(true)), Resumed::All));
    }

    #[test]
    fn a_shell_that_never_ran_may_be_started_again() {
        assert!(matches!(
            resume_all(vec![7], |_| Err(refused())),
            Resumed::NotStarted(_)
        ));
        assert!(matches!(
            resume_all(Vec::new(), |_| Ok(true)),
            Resumed::NotStarted(_)
        ));
        // A thread ID reused by another process resumes nothing of the shell.
        let reused = resume_all(vec![7, 8], |thread| {
            if thread == 7 {
                Ok(false)
            } else {
                Err(refused())
            }
        });
        assert!(matches!(reused, Resumed::NotStarted(_)));
    }

    #[test]
    fn a_shell_with_a_running_thread_is_never_started_again() {
        let mut tried = Vec::new();
        let resumed = resume_all(vec![7, 8, 9], |thread| {
            tried.push(thread);
            if thread == 7 {
                Ok(true)
            } else {
                Err(refused())
            }
        });
        assert!(matches!(resumed, Resumed::Partly(_)));
        assert_eq!(tried, [7, 8], "resuming stops at the first failure");
    }
}
