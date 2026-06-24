use std::ffi::OsString;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut target = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("coven-cave: failed to locate executable path: {error}");
            return ExitCode::FAILURE;
        }
    };

    target.set_file_name(if cfg!(windows) {
        "coven-code.exe"
    } else {
        "coven-code"
    });

    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match Command::new(&target).args(args).status() {
        Ok(status) => match status.code() {
            Some(0) => ExitCode::SUCCESS,
            Some(code) if (1..=255).contains(&code) => ExitCode::from(code as u8),
            _ => ExitCode::FAILURE,
        },
        Err(error) => {
            eprintln!("coven-cave: failed to run {}: {error}", target.display());
            ExitCode::FAILURE
        }
    }
}
