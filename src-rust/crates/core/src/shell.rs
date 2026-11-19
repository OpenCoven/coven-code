//! The platform shell that runs a command line given as one string.

use std::process::Command;

/// A command that runs `line` in the platform's shell: `sh -c` on Unix and
/// `cmd.exe` on Windows.
///
/// `cmd.exe` does not read its command line by the C runtime's rules, which
/// are the ones `Command::arg` writes: a `"` inside an argument would reach
/// it as `\"` (#517). So on Windows the line is appended unescaped, inside
/// one more pair of quotes. `cmd.exe` strips the first and the last quote of
/// what follows `/C`, which are that pair, and takes the rest as written.
///
/// It keeps the pair in one case: the line has no quote of its own, none of
/// `&<>()@^|`, and is the path, with a space in it, of a program. Then it
/// runs that program. `/S` would turn that off, and is left out: such a line
/// has always run here, because `Command::arg` put the same pair around it.
///
/// So `cmd.exe` reads a line without a quote as it did when the line came
/// through `Command::arg`, but for a backslash at its very end, which is no
/// longer doubled.
///
/// `line` is the whole command line. Add no arguments to the returned
/// command: on Windows they would land after the closing quote.
pub fn command(line: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut shell = Command::new("cmd");
        shell.raw_arg(format!("/C \"{line}\""));
        shell
    }
    #[cfg(not(windows))]
    {
        let mut shell = Command::new("sh");
        shell.args(["-c", line]);
        shell
    }
}

/// The variable through which [`start`] hands `cmd.exe` its target.
#[cfg(windows)]
pub const START_TARGET_VARIABLE: &str = "COVEN_CODE_START_TARGET";

/// Opens `target`, a file or a URL, with the program Windows associates
/// with it: `shell`, a `cmd.exe` with its environment already set, runs
/// `start "" "<target>"` (#517).
///
/// The target is not written into the command line. `cmd.exe` replaces
/// `%NAME%` there with a variable's value before it reads quotes, and a
/// value can hold a `"`, which would end the quoting and let what follows
/// run as commands. Instead the line is `start "" "!VARIABLE!"`, appended
/// as written like [`command`]'s, with delayed expansion on (`/V:ON`) and
/// the target in [`START_TARGET_VARIABLE`]. `cmd.exe` puts that value in
/// after it has split the line into commands and expanded `%NAME%`, and
/// scans it for neither. `start` takes the first quoted argument as a
/// window title, hence the empty one. `/D` skips the user's `AutoRun`
/// commands. The opened program inherits the variable, so a browser started
/// for an OAuth login keeps the authorization URL in its environment.
///
/// A target with a `"` in it is refused with `InvalidInput`, because
/// `start` would split it there. Windows paths cannot contain one, and
/// `url::Url` writes it percent-encoded. An empty target is refused too:
/// `start ""` would open a console window.
///
/// Add no arguments to the returned command, and do not clear its
/// environment.
#[cfg(windows)]
pub fn start(mut shell: Command, target: impl AsRef<std::ffi::OsStr>) -> std::io::Result<Command> {
    use std::os::windows::{ffi::OsStrExt, process::CommandExt};
    let target = target.as_ref();
    if target.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "no target to open",
        ));
    }
    if target.encode_wide().any(|unit| unit == u16::from(b'"')) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "a target to open contains a double quote",
        ));
    }
    shell.env(START_TARGET_VARIABLE, target).raw_arg(format!(
        "/D /V:ON /C \"start \"\" \"!{START_TARGET_VARIABLE}!\"\""
    ));
    Ok(shell)
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn unix_runs_the_line_with_sh() {
        let line = r#"echo "a b" && echo 'c'"#;
        let shell = super::command(line);
        assert_eq!(shell.get_program(), "sh");
        assert_eq!(shell.get_args().collect::<Vec<_>>(), ["-c", line]);
    }
}
