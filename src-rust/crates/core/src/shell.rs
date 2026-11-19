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
        cmd_c("cmd".as_ref(), line.as_ref())
    }
    #[cfg(not(windows))]
    {
        let mut shell = Command::new("sh");
        shell.args(["-c", line]);
        shell
    }
}

/// A command that opens `target`, a file or a URL, with the program Windows
/// associates with it: `cmd` (a `cmd.exe`) runs `start "" "<target>"`.
///
/// The line reaches `cmd.exe` as [`command`] hands it over, so the quotes
/// around `target` arrive as written (#517) and `cmd.exe` reads a space,
/// `&`, `|`, `<`, `>` or `^` in it as text. `start` takes the first quoted
/// argument as a window title, hence the empty one.
///
/// A target with a `"` in it is refused with `InvalidInput`: it would end
/// the quoting, and what followed would be read as commands. Windows paths
/// cannot contain one, and `url::Url` writes it percent-encoded. `cmd.exe`
/// still expands `%NAME%` when `NAME` is a set environment variable.
///
/// Add no arguments to the returned command.
#[cfg(windows)]
pub fn start(
    cmd: impl AsRef<std::ffi::OsStr>,
    target: impl AsRef<std::ffi::OsStr>,
) -> std::io::Result<Command> {
    use std::os::windows::ffi::OsStrExt;
    let target = target.as_ref();
    if target.encode_wide().any(|unit| unit == u16::from(b'"')) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "a target to open contains a double quote",
        ));
    }
    let mut line = std::ffi::OsString::from("start \"\" \"");
    line.push(target);
    line.push("\"");
    Ok(cmd_c(cmd.as_ref(), &line))
}

/// `program /C "<line>"`, with `line` appended as written.
#[cfg(windows)]
fn cmd_c(program: &std::ffi::OsStr, line: &std::ffi::OsStr) -> Command {
    use std::os::windows::process::CommandExt;
    let mut argument = std::ffi::OsString::from("/C \"");
    argument.push(line);
    argument.push("\"");
    let mut shell = Command::new(program);
    shell.raw_arg(argument);
    shell
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
