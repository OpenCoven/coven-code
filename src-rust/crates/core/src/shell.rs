//! The platform shell that runs a command line given as one string.

use std::process::Command;

/// A command that runs `line` in the platform's shell: `sh -c` on Unix and
/// `cmd.exe` on Windows.
///
/// `cmd.exe` does not read its command line by the C runtime's rules, which
/// are the ones `Command::arg` writes: a `"` inside an argument would reach
/// it as `\"` (#517). So on Windows the line is appended unescaped, inside
/// one more pair of quotes, and `/S` has `cmd.exe` strip exactly that pair
/// and take the rest as written.
///
/// `line` is the whole command line. Add no arguments to the returned
/// command: on Windows they would land after the closing quote.
pub fn command(line: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut shell = Command::new("cmd");
        shell.raw_arg(format!("/S /C \"{line}\""));
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
