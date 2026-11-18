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
/// It keeps the pair in one case: the line has no quote of its own and is
/// the path, with a space in it, of a program. Then it runs that program.
/// `/S` would turn that off, and is left out: such a line has always run
/// here, because `Command::arg` put the same pair around it.
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
