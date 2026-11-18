//! Host-owned, single-process scope for headless Chat turns. Kernel admission
//! checks the actual Seatbelt profile before the engine starts model work.
use claurst_core::permissions::{
    PermissionDecision, PermissionHandler, PermissionManager, PermissionRequest,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
    time::Duration,
};

const MAX_POLICY_BYTES: u64 = 65_536;
static ACTIVE: OnceLock<WorkspaceScope> = OnceLock::new();
pub const KERNEL_PROBE: &str = "--workspace-kernel-probe-v1";
pub const VERIFY_POLICY: &str = "--verify-workspace-policy-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceScope {
    version: u32,
    run_id: String,
    access: String,
    primary_root: PathBuf,
    writable_roots: Vec<PathBuf>,
    read_only_roots: Vec<PathBuf>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

impl WorkspaceScope {
    pub fn read(file: &Path, cwd: &Path) -> io::Result<Self> {
        if !file.symlink_metadata()?.is_file() {
            return Err(invalid("Workspace access requires a regular policy file."));
        }
        let mut bytes = Vec::new();
        fs::File::open(file)?
            .take(MAX_POLICY_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_POLICY_BYTES {
            return Err(invalid("Workspace access policy exceeds its size limit."));
        }
        let policy: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        policy.validate(cwd, file)?;
        Ok(policy)
    }

    fn validate(&self, cwd: &Path, file: &Path) -> io::Result<()> {
        if self.version != 1
            || self.access != "workspace-write"
            || self.run_id.is_empty()
            || self.run_id.len() > 128
            || !self
                .run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            || self.writable_roots.is_empty()
            || self.writable_roots.len() > 128
            || self.read_only_roots.len() > 128
            || self.primary_root != cwd.canonicalize()?
        {
            return Err(invalid("Invalid workspace access scope for this run."));
        }
        let policy_path = file.canonicalize()?;
        let home = dirs::home_dir().and_then(|path| path.canonicalize().ok());
        for roots in [&self.writable_roots, &self.read_only_roots] {
            if roots.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(invalid("Workspace access roots must be sorted and unique."));
            }
            for root in roots {
                if !root.is_absolute()
                    || root.parent().is_none()
                    || !root.is_dir()
                    || root.canonicalize()? != *root
                    || home.as_ref() == Some(root)
                    || root
                        .to_str()
                        .is_none_or(|path| path.chars().any(char::is_control))
                {
                    return Err(invalid(
                        "Workspace access requires existing canonical dedicated folders.",
                    ));
                }
            }
        }
        if !self
            .writable_roots
            .iter()
            .any(|root| self.primary_root.starts_with(root))
            || self
                .writable_roots
                .iter()
                .any(|root| policy_path.starts_with(root))
            || self.read_only_roots.iter().any(|read| {
                self.writable_roots
                    .iter()
                    .any(|write| read.starts_with(write))
            })
        {
            return Err(invalid(
                "Workspace access overlaps a protected or read-only folder.",
            ));
        }
        Ok(())
    }

    /// The profile and parameter bindings used by the actual shell launch.
    fn command_parts(
        &self,
        executable: &Path,
        args: &[String],
    ) -> io::Result<(PathBuf, Vec<String>)> {
        if !cfg!(target_os = "macos") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "This Coven Code build cannot establish a workspace sandbox on this platform.",
            ));
        }
        let mut profile = String::from(include_str!("workspace_scope/seatbelt.sbpl"));
        profile.push_str("\n(allow file-read*)\n");
        let mut argv = Vec::new();
        for (i, root) in self.writable_roots.iter().enumerate() {
            // Only an unchanged canonical directory launches; every other
            // outcome of this check is a refusal.
            let unchanged = match root.canonicalize() {
                Ok(resolved) => resolved == *root && root.is_dir(),
                // Renamed or removed since admission: nothing resolves at the
                // saved path, or a folder above it is no longer a directory.
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) =>
                {
                    false
                }
                // Any other failure (the folder may be intact but unreachable)
                // names this check instead of passing on a bare OS error.
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "A saved workspace folder could not be checked before launch: {error}"
                        ),
                    ));
                }
            };
            if !unchanged {
                return Err(invalid("A saved workspace folder changed before launch."));
            }
            let key = format!("COVEN_WRITE_{i}");
            profile.push_str(&format!(
                "(allow file-write* (subpath (param \"{key}\")))\n(deny file-write-unlink (literal (param \"{key}\")))\n"
            ));
            argv.extend([
                "-D".into(),
                format!(
                    "{key}={}",
                    root.to_str()
                        .ok_or_else(|| invalid("Invalid workspace folder path."))?
                ),
            ]);
        }
        argv.extend([
            "-p".into(),
            profile,
            executable
                .to_str()
                .ok_or_else(|| invalid("Invalid sandbox command path."))?
                .into(),
        ]);
        argv.extend_from_slice(args);
        Ok((PathBuf::from("/usr/bin/sandbox-exec"), argv))
    }

    pub async fn verify(self, policy_file: &Path) -> io::Result<VerifiedScope> {
        self.validate(&self.primary_root, policy_file)?;
        let mut args = vec![KERNEL_PROBE.into(), self.writable_roots.len().to_string()];
        let outside = policy_file.canonicalize()?;
        for path in self
            .writable_roots
            .iter()
            .chain(self.read_only_roots.iter())
            .chain(std::iter::once(&outside))
        {
            args.push(
                path.to_str()
                    .ok_or_else(|| invalid("Invalid workspace folder path."))?
                    .into(),
            );
        }
        let (program, argv) = self.command_parts(&std::env::current_exe()?, &args)?;
        let status = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new(program)
                .args(argv)
                .current_dir(&self.primary_root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .kill_on_drop(true)
                .status(),
        )
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Workspace sandbox verification timed out.",
            )
        })??;
        if !status.success() {
            return Err(io::Error::other("The workspace sandbox did not allow exactly the saved folders. Check folder access and update Coven Code before retrying."));
        }
        Ok(VerifiedScope(self))
    }

    fn decision(
        &self,
        request: &PermissionRequest,
        prior: PermissionDecision,
    ) -> PermissionDecision {
        // A scoped grant never overrides a native deny rule.
        if matches!(prior, PermissionDecision::Deny) {
            return prior;
        }
        match request.tool_name.as_str() {
            "Bash" => PermissionDecision::Allow,
            "Write" | "Edit" | "ApplyPatch" | "BatchEdit" | "NotebookEdit" => {
                if request
                    .path
                    .as_ref()
                    .and_then(|path| super::canonicalize_path_allow_missing(Path::new(path)).ok())
                    .is_some_and(|path| {
                        self.writable_roots
                            .iter()
                            .any(|root| path.starts_with(root))
                    })
                {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Ask { reason: "This file is outside the folders Chat allowed for this run. Ask for the folder explicitly in Chat.".into() }
                }
            }
            "Read" | "Grep" | "Glob" => prior,
            _ => PermissionDecision::Ask {
                reason: "This tool needs access beyond this workspace run.".into(),
            },
        }
    }
}

/// Only a successful kernel query can construct this admission token.
pub struct VerifiedScope(WorkspaceScope);
impl VerifiedScope {
    fn receipt(&self) -> io::Result<serde_json::Value> {
        let mut receipt = serde_json::to_value(&self.0).map_err(io::Error::other)?;
        receipt["enforced"] = serde_json::Value::Bool(true);
        Ok(receipt)
    }
    pub fn write_receipt(&self, out: impl io::Write) -> io::Result<()> {
        serde_json::to_writer(out, &self.receipt()?).map_err(io::Error::other)
    }
    pub fn emit_admission(&self, cwd: &Path, mut out: impl io::Write) -> io::Result<()> {
        serde_json::to_writer(
            &mut out,
            &serde_json::json!({
                "type": "system",
                "subtype": "init",
                "cwd": cwd,
                "permission": "workspace-write",
                "tools": [],
                "effectiveAccess": self.receipt()?,
            }),
        )
        .map_err(io::Error::other)?;
        out.write_all(b"\n")?;
        out.flush()
    }
    pub fn install(self) -> io::Result<()> {
        ACTIVE
            .set(self.0)
            .map_err(|_| invalid("Workspace access is already bound to a different run."))
    }
}

/// The real policy query runs inside the same kernel profile used for shells.
/// Checking existing directories avoids a false denial for an unresolved path.
#[cfg(target_os = "macos")]
pub fn kernel_probe(args: &[String]) -> io::Result<()> {
    use nix::libc;
    use std::ffi::CString;
    unsafe extern "C" {
        fn sandbox_check(
            pid: libc::pid_t,
            operation: *const libc::c_char,
            filter: libc::c_int,
            ...
        ) -> libc::c_int;
    }
    let (count, paths) = args
        .split_first()
        .ok_or_else(|| invalid("Missing sandbox probe roots."))?;
    let count = count.parse::<usize>().map_err(io::Error::other)?;
    if count == 0 || count >= paths.len() {
        return Err(invalid("Invalid sandbox probe root count."));
    }
    verify_process_group_boundary()?;
    for (i, path) in paths.iter().enumerate() {
        let path = CString::new(path.as_str()).map_err(io::Error::other)?;
        // SAFETY: both strings are NUL-terminated and live through the call;
        // filter 1 is SANDBOX_FILTER_PATH. Query only this process's profile.
        let result = unsafe {
            sandbox_check(
                libc::getpid(),
                c"file-write-data".as_ptr(),
                1,
                path.as_ptr(),
            )
        };
        // SAFETY: path is NUL-terminated. In addition to the sandbox
        // decision, check the real UID's access without creating a file.
        let filesystem_denied =
            i < count && unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) } != 0;
        if result != i32::from(i >= count) {
            return Err(io::Error::other(format!(
                "Workspace sandbox returned an unexpected file policy for root {i}."
            )));
        }
        if filesystem_denied {
            return Err(io::Error::other(format!(
                "Workspace sandbox could not write to allowed folder {i}: {}",
                io::Error::last_os_error()
            )));
        }
    }
    Ok(())
}

/// A pathname sandbox alone cannot keep a child in Coven's process group.
#[cfg(target_os = "macos")]
fn verify_process_group_boundary() -> io::Result<()> {
    use nix::libc;
    let refused =
        |result| result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    // SAFETY: these syscalls act only on the short-lived probe process.
    if !refused(unsafe { libc::setsid() }) {
        return Err(io::Error::other(
            "The workspace sandbox did not refuse a new process session.",
        ));
    }
    // SAFETY: the zero arguments mean this process and a group with its own ID.
    if !refused(unsafe { libc::setpgid(0, 0) }) {
        return Err(io::Error::other(
            "The workspace sandbox did not refuse a new process group.",
        ));
    }
    let mut child = 0;
    let program = c"/usr/bin/true".as_ptr();
    let argv = [program.cast_mut(), std::ptr::null_mut()];
    let environment = [std::ptr::null_mut()];
    // SAFETY: program, argument and environment pointers live until
    // posix_spawn returns. Both arrays are null-terminated. If permitted
    // unexpectedly, reap the harmless child before rejecting the policy.
    let status = unsafe {
        libc::posix_spawn(
            &mut child,
            program,
            std::ptr::null(),
            std::ptr::null(),
            argv.as_ptr(),
            environment.as_ptr(),
        )
    };
    if status == 0 {
        // SAFETY: child is the owned, successfully launched process.
        unsafe { libc::waitpid(child, std::ptr::null_mut(), 0) };
    }
    if status != libc::EPERM {
        return Err(io::Error::other(format!(
            "The workspace sandbox did not refuse posix_spawn (returned {status})."
        )));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn kernel_probe(_args: &[String]) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Workspace sandbox verification is unavailable on this platform.",
    ))
}

/// Keep explicit native deny rules distinct from a question. The ordinary
/// unattended handler collapses both to Deny before the host scope can decide.
pub struct ScopedPermissionHandler(pub std::sync::Arc<std::sync::Mutex<PermissionManager>>);

impl PermissionHandler for ScopedPermissionHandler {
    fn check_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        let Ok(manager) = self.0.lock() else {
            return PermissionDecision::Deny;
        };
        manager.evaluate(
            &request.tool_name,
            &request.description,
            request.path.as_deref(),
            request.working_dir.as_deref(),
            &request.allowed_roots,
        )
    }

    fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.check_permission(request)
    }
}

pub fn additional_roots() -> Vec<PathBuf> {
    ACTIVE.get().map_or_else(Vec::new, |scope| {
        scope
            .writable_roots
            .iter()
            .chain(&scope.read_only_roots)
            .cloned()
            .collect()
    })
}

pub fn active() -> bool {
    ACTIVE.get().is_some()
}

pub(crate) fn apply(request: &PermissionRequest, prior: PermissionDecision) -> PermissionDecision {
    match ACTIVE.get() {
        Some(scope) => scope.decision(request, prior),
        None => prior,
    }
}

pub(crate) fn shell(program: &str, args: &[String]) -> io::Result<(PathBuf, Vec<String>)> {
    match ACTIVE.get() {
        Some(scope) if program == "bash" => scope.command_parts(Path::new("/bin/bash"), args),
        Some(scope) => scope.command_parts(Path::new(program), args),
        None => Ok((PathBuf::from(program), args.to_vec())),
    }
}

pub(crate) fn permits_tool(name: &str) -> bool {
    matches!(
        name,
        "Bash"
            | "Read"
            | "Grep"
            | "Glob"
            | "Write"
            | "Edit"
            | "ApplyPatch"
            | "BatchEdit"
            | "NotebookEdit"
            | "TaskList"
            | "TaskGet"
            | "TaskOutput"
            | "TaskStop"
    )
}

async fn mutate(program: &str, args: &[String], input: Option<&[u8]>) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let (program, args) = shell(program, args)?;
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    if let Some(bytes) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("Workspace write input is unavailable."))?
            .write_all(bytes)
            .await?;
    }
    if !child.wait().await?.success() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "The workspace sandbox refused this file change.",
        ));
    }
    Ok(())
}

pub(crate) async fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    let path = path.as_ref();
    if !active() {
        return tokio::fs::write(path, bytes).await;
    }
    let path = path
        .to_str()
        .ok_or_else(|| invalid("Invalid workspace file path."))?;
    mutate(
        "/bin/sh",
        &[
            "-c".into(),
            r#"/bin/cat > "$1""#.into(),
            "workspace-write".into(),
            path.into(),
        ],
        Some(bytes.as_ref()),
    )
    .await
}

pub(crate) async fn create_dir_all(path: &Path) -> io::Result<()> {
    if !active() {
        return tokio::fs::create_dir_all(path).await;
    }
    let path = path
        .to_str()
        .ok_or_else(|| invalid("Invalid workspace folder path."))?;
    mutate("/bin/mkdir", &["-p".into(), "--".into(), path.into()], None).await
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn native_shell_writes_only_to_workspace_and_explicit_project() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let work = base.join("a-workspace");
        let project = base.join("b-project");
        let read = base.join("c-read-only");
        let outside = base.join("d-ungranted");
        for root in [&work, &project, &read, &outside] {
            fs::create_dir(root).unwrap();
        }
        symlink(&outside, work.join("alias")).unwrap();
        let scope = WorkspaceScope {
            version: 1,
            run_id: "test".into(),
            access: "workspace-write".into(),
            primary_root: work.clone(),
            writable_roots: vec![work.clone(), project.clone()],
            read_only_roots: vec![read.clone()],
        };
        let outside_file = outside.join("protected");
        fs::write(&outside_file, "protected").unwrap();
        let (program, argv) = scope
            .command_parts(
                Path::new("/bin/ln"),
                &[
                    outside_file.to_str().unwrap().into(),
                    work.join("hardlink").to_str().unwrap().into(),
                ],
            )
            .unwrap();
        let output = std::process::Command::new(program)
            .args(argv)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(!work.join("hardlink").exists());
        for (file, allowed) in [
            (work.join("ok"), true),
            (project.join("ok"), true),
            (read.join("blocked"), false),
            (outside.join("blocked"), false),
            (work.join("alias/blocked"), false),
        ] {
            let (program, argv) = scope
                .command_parts(
                    Path::new("/bin/sh"),
                    &[
                        "-c".into(),
                        "printf scoped > \"$1\"".into(),
                        "test".into(),
                        file.to_str().unwrap().into(),
                    ],
                )
                .unwrap();
            let output = std::process::Command::new(program)
                .args(argv)
                .current_dir(&work)
                .output()
                .unwrap();
            assert_eq!(output.status.success(), allowed, "{file:?}: {:?}", output);
            assert_eq!(file.exists(), allowed, "{file:?}");
        }
    }

    const CHANGED: &str = "A saved workspace folder changed before launch.";
    const UNCHECKED: &str = "A saved workspace folder could not be checked before launch: ";

    /// A scope over `work` and `project`, both writable, and its pre-launch
    /// check as a shell launch would run it.
    fn launch_check(work: &Path, project: &Path) -> impl Fn() -> io::Result<()> {
        let scope = WorkspaceScope {
            version: 1,
            run_id: "test".into(),
            access: "workspace-write".into(),
            primary_root: work.to_path_buf(),
            writable_roots: vec![work.to_path_buf(), project.to_path_buf()],
            read_only_roots: vec![],
        };
        move || {
            scope
                .command_parts(Path::new("/bin/sh"), &["-c".into(), "true".into()])
                .map(|_| ())
        }
    }

    /// #531: a granted folder that is no longer at its saved path refuses the
    /// launch with the changed-folder message, not a bare resolve error.
    #[test]
    fn a_writable_root_missing_from_its_saved_path_reads_as_changed() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let work = base.join("a-workspace");
        let holder = base.join("b-holder");
        let project = holder.join("project");
        fs::create_dir(&work).unwrap();
        fs::create_dir_all(&project).unwrap();
        let launch = launch_check(&work, &project);
        launch().expect("unchanged folders launch");
        let changed = |state: &str| {
            let error = launch().expect_err(state);
            assert_eq!(
                error.kind(),
                io::ErrorKind::InvalidInput,
                "{state}: {error}"
            );
            assert_eq!(error.to_string(), CHANGED, "{state}");
        };

        let moved = holder.join("moved");
        fs::rename(&project, &moved).unwrap();
        changed("renamed away");

        symlink(holder.join("nowhere"), &project).unwrap();
        changed("replaced by a dangling symlink");
        fs::remove_file(&project).unwrap();

        symlink(&moved, &project).unwrap();
        changed("replaced by a symlink to another folder");
        fs::remove_file(&project).unwrap();

        fs::write(&project, "").unwrap();
        changed("replaced by a regular file");
        fs::remove_file(&project).unwrap();

        // The folder that held the project is now a file, so the saved path
        // fails to resolve with "not a directory" instead of "not found".
        fs::rename(&holder, base.join("c-held")).unwrap();
        fs::write(&holder, "").unwrap();
        changed("parent replaced by a regular file");
        fs::remove_file(&holder).unwrap();

        fs::rename(base.join("c-held"), &holder).unwrap();
        fs::rename(&moved, &project).unwrap();
        launch().expect("the restored folder launches again");
    }

    /// A resolve failure that does not show the folder is gone still refuses
    /// the launch. It names the check and keeps the cause, and does not claim
    /// that the folder changed.
    #[test]
    fn a_writable_root_that_cannot_be_checked_still_refuses_the_launch() {
        use std::os::unix::fs::PermissionsExt;
        // Root's path lookups ignore mode bits, which the first case relies on.
        assert!(
            !nix::unistd::getuid().is_root(),
            "run this suite as an ordinary user"
        );
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let work = base.join("a-workspace");
        let holder = base.join("b-holder");
        let project = holder.join("project");
        fs::create_dir(&work).unwrap();
        fs::create_dir_all(&project).unwrap();
        let launch = launch_check(&work, &project);
        launch().expect("unchanged folders launch");

        // The project is intact but its parent can no longer be searched.
        fs::set_permissions(&holder, fs::Permissions::from_mode(0o000)).unwrap();
        let denied = launch();
        fs::set_permissions(&holder, fs::Permissions::from_mode(0o755)).unwrap();
        let error = denied.expect_err("an unreadable parent");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert_eq!(
            error.to_string(),
            format!("{UNCHECKED}Permission denied (os error 13)")
        );

        // A symlink to itself: the path exists but never resolves. Its error
        // (ELOOP) has no stable io::ErrorKind, so only the refusal and the
        // wording are pinned, not the kind.
        fs::remove_dir(&project).unwrap();
        symlink(&project, &project).unwrap();
        let error = launch().expect_err("a symlink loop");
        assert_ne!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
        assert!(
            error.to_string().starts_with(UNCHECKED) && error.to_string() != UNCHECKED,
            "{error}"
        );
    }
}
