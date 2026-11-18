//! A loop for the Windows unit tests of commands that must be stopped with
//! everything they started (#373). Only `cmd.exe` runs it: each pass appends a
//! line to a file, then waits about a second in `ping`. The file stops growing
//! once that `cmd.exe` is gone.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// A directory holding `ticker.cmd`, which appends to `ticks.txt` beside it
/// by absolute path, so it ticks into the same file from any directory.
pub(crate) struct Ticker {
    directory: tempfile::TempDir,
}

impl Ticker {
    pub(crate) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let ticks = directory.path().join("ticks.txt");
        // The echoed word has no digits, since `echo 1>>file` redirects
        // handle 1. A batch file needs `%%i` where the command line takes `%i`.
        let script = format!(
            "@for /L %%i in (1,1,60) do @(echo tick>>\"{}\" & ping -n 2 127.0.0.1 >nul)\r\n",
            ticks.display()
        );
        std::fs::write(directory.path().join("ticker.cmd"), script).unwrap();
        Self { directory }
    }

    pub(crate) fn directory(&self) -> &Path {
        self.directory.path()
    }

    pub(crate) fn script(&self) -> PathBuf {
        self.directory.path().join("ticker.cmd")
    }

    /// Lines in `ticks.txt`. A read that collides with `cmd.exe` appending is
    /// retried rather than counted as empty.
    pub(crate) fn ticks(&self) -> usize {
        let path = self.directory.path().join("ticks.txt");
        for _ in 0..40 {
            match std::fs::read_to_string(&path) {
                Ok(text) => return text.lines().count(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return 0,
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
        panic!("could not read {}", path.display());
    }

    /// Panics unless the loop ticked, and then stopped: the count 3 s after
    /// the call ended, when a pass already under way has landed, must equal
    /// the count 3 s later, when a live loop would have added about three.
    pub(crate) async fn assert_stopped(&self, what: &str) {
        let stopped = self.ticks();
        assert!(stopped >= 1, "{what}: the loop never started");
        tokio::time::sleep(Duration::from_secs(3)).await;
        let settled = self.ticks();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let later = self.ticks();
        assert_eq!(
            settled, later,
            "{what}: the loop kept running after the call ended: \
             {stopped} ticks then, {settled} 3 s later, {later} 6 s later"
        );
    }
}
