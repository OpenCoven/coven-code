//! A Job Object for each foreground Windows shell command (#373).
//!
//! Killing `cmd.exe` or `powershell.exe` does not kill what it started. So the
//! shell is created suspended, joins a new job, and only then runs: everything
//! it starts is created inside the job. Terminating the job, or closing its
//! only handle, kills all of it.

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
    OpenThread, ResumeThread, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
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

/// Spawn `command` with its whole process tree in a new job.
///
/// The shell is created suspended and resumed only once it is in the job, so
/// it cannot start anything outside it. If no job can be created or joined,
/// for example under a parent job that does not allow nesting, the command
/// still runs, and cancel and timeout stop only the shell, as before. Only a
/// failure to start the shell at all is an error.
pub(crate) fn spawn(command: &mut Command) -> io::Result<(Child, Option<Job>)> {
    let job = match Job::new() {
        Ok(job) => job,
        Err(error) => {
            tracing::debug!(%error, "No job for a foreground command; only its shell will be stopped");
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
        tracing::debug!(%error, "Could not put a foreground command in a job; only its shell will be stopped");
        None
    };
    if let Err(error) = resume(id) {
        // The suspended shell has not run any code yet, so start it again.
        tracing::debug!(%error, "Could not resume a foreground command's shell; starting it again without a job");
        let _ = child.start_kill();
        return Ok((command.creation_flags(0).spawn()?, None));
    }
    Ok((child, job))
}

/// Resume the primary thread of `process_id`, created with `CREATE_SUSPENDED`.
/// std closes that thread's handle after creating the process, so find the
/// thread in a snapshot.
fn resume(process_id: u32) -> io::Result<()> {
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
    let mut resumed = false;
    // SAFETY: `entry` is a THREADENTRY32 with `dwSize` set, as the walk needs.
    let mut listed = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
    while listed {
        if entry.th32OwnerProcessID == process_id {
            // SAFETY: takes no pointers.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `thread` is a new, open handle that nothing else owns.
            let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
            // SAFETY: `thread` was opened with THREAD_SUSPEND_RESUME.
            if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            resumed = true;
        }
        // SAFETY: as for Thread32First.
        listed = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
    }
    if resumed {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the suspended shell has no thread to resume",
        ))
    }
}
