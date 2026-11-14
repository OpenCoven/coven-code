//! Bounded child-output collection for interactive diagnostics. Never write child
//! bytes to a terminal, and never wait for an unterminated line or descendant EOF.

use std::collections::VecDeque;
use std::io;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const RETAIN_BYTES: usize = 8192;
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// A bounded tail for one reporting interval, including an explicit loss count.
#[derive(Default)]
struct Tail {
    bytes: VecDeque<u8>,
    omitted: u64,
}

impl Tail {
    fn push(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if self.bytes.len() == RETAIN_BYTES {
                self.bytes.pop_front();
                self.omitted = self.omitted.saturating_add(1);
            }
            self.bytes.push_back(*byte);
        }
    }

    fn report(&mut self, source: &str) {
        if self.bytes.is_empty() {
            return;
        }
        // Debug escaping renders ALL controls inert, including C1, invalid UTF-8,
        // newlines and Unicode bidi formatting. Bound input before formatting.
        let text = format!(
            "{:?}",
            String::from_utf8_lossy(self.bytes.make_contiguous())
        );
        tracing::warn!(target: "claurst_core::child_diagnostics", source, omitted_bytes = self.omitted, output = %text, "Child output");
        self.bytes.clear();
        self.omitted = 0;
    }
}

/// Owns a continuously draining pipe. Finish after retiring the child; it joins
/// the reader even when descendants retain pipe handles. Drop aborts as fallback.
pub struct DiagnosticReader {
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<io::Result<()>>>,
}

impl DiagnosticReader {
    pub fn new(reader: impl AsyncRead + Unpin + Send + 'static, source: &str) -> Self {
        // Server names and paths may themselves contain hostile controls.
        let source = format!("{:?}", source.chars().take(256).collect::<String>());
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(collect(reader, source, stopped));
        Self {
            stop: Some(stop),
            task: Some(task),
        }
    }

    pub async fn finish(&mut self) -> io::Result<()> {
        let Some(task) = self.task.as_mut() else {
            return Ok(());
        };
        // Leave time to drain the final writes after exit, without depending on
        // EOF from an unrelated descendant that inherited the pipe.
        let result = match tokio::time::timeout(Duration::from_millis(100), &mut *task).await {
            Ok(result) => result,
            Err(_) => {
                if let Some(stop) = self.stop.take() {
                    let _ = stop.send(());
                }
                task.await
            }
        };
        self.task.take();
        result.map_err(io::Error::other)?
    }
}

/// A system opener is an owned, short-lived launcher. The launched GUI keeps its
/// own lifetime, but its command cannot inherit the interactive terminal.
pub struct ExternalOpener {
    task: JoinHandle<()>,
}

impl ExternalOpener {
    pub fn spawn(mut command: tokio::process::Command) -> io::Result<Self> {
        use std::process::Stdio;
        // Check before spawning: a missing runtime must not leave a child behind.
        let runtime = tokio::runtime::Handle::try_current().map_err(io::Error::other)?;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdout = child
            .stdout
            .take()
            .map(|pipe| DiagnosticReader::new(pipe, "external opener stdout"));
        let mut stderr = child
            .stderr
            .take()
            .map(|pipe| DiagnosticReader::new(pipe, "external opener stderr"));
        let task = runtime.spawn(async move {
            let status = match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
                Ok(status) => status,
                Err(_) => {
                    tracing::warn!(target: "claurst_core::child_diagnostics", "External opener timed out");
                    child.kill().await
                        .and(Err(io::Error::new(io::ErrorKind::TimedOut, "External opener timed out")))
                }
            };
            match status {
                Ok(status) if status.success() => {},
                Ok(status) => tracing::warn!(target: "claurst_core::child_diagnostics", %status, "External opener failed"),
                Err(error) => tracing::warn!(target: "claurst_core::child_diagnostics", kind = ?error.kind(), "External opener failed"),
            }
            if let Some(reader) = &mut stdout { let _ = reader.finish().await; }
            if let Some(reader) = &mut stderr { let _ = reader.finish().await; }
        });
        Ok(Self { task })
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for ExternalOpener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Drop for DiagnosticReader {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn collect(
    mut reader: impl AsyncRead + Unpin,
    source: String,
    mut stopped: oneshot::Receiver<()>,
) -> io::Result<()> {
    let mut tail = Tail::default();
    let mut buffer = [0; 4096];
    let mut tick = tokio::time::interval(REPORT_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        tokio::select! {
            biased;
            _ = &mut stopped => break Ok(()),
            _ = tick.tick() => tail.report(&source),
            read = reader.read(&mut buffer) => match read {
                Ok(0) => break Ok(()),
                Ok(n) => tail.push(&buffer[..n]),
                Err(error) => break Err(error),
            },
        }
        // Ready pipes must yield so a flood cannot starve protocol or cancellation.
        tokio::task::yield_now().await;
    };
    tail.report(&source);
    if let Err(error) = &result {
        tracing::warn!(target: "claurst_core::child_diagnostics", source, kind = ?error.kind(), "Child output reader failed");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tokio::io::AsyncWriteExt;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn diagnostic_events_escape_controls_and_mark_truncation() {
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let mut tail = Tail::default();
            tail.push(&vec![b'x'; RETAIN_BYTES + 1]);
            tail.push("last\x1b]52;clipboard\x07\n\r\x00\u{009b}\u{202e}".as_bytes());
            tail.push(&[0xff]);
            tail.report("safe source");
            assert!(tail.bytes.is_empty());
            assert_eq!(tail.omitted, 0);
        });
        let text = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(
            text.contains("last")
                && text.contains("omitted_bytes=")
                && text.contains("Child output")
        );
        for control in ['\x1b', '\x07', '\r', '\x00', '\u{009b}', '\u{202e}'] {
            assert!(!text.contains(control), "raw control {control:?}");
        }
        assert_eq!(
            text.lines().count(),
            1,
            "child newlines cannot forge events"
        );
        assert!(text.len() < RETAIN_BYTES * 7);
    }

    #[tokio::test]
    async fn reader_failure_is_returned_after_join() {
        struct Failed;
        impl AsyncRead for Failed {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
                _: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<io::Result<()>> {
                std::task::Poll::Ready(Err(io::Error::other("fixture")))
            }
        }
        let mut reader = DiagnosticReader::new(Failed, "fixture");
        assert!(reader.finish().await.is_err());
        assert!(reader.task.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn opener_drains_both_pipes_and_reaps_failure() {
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "dd if=/dev/zero bs=4096 count=100 2>/dev/null; dd if=/dev/zero bs=4096 count=100 1>&2 2>/dev/null; exit 7"]);
        let mut opener = ExternalOpener::spawn(command).unwrap();
        tokio::time::timeout(Duration::from_secs(5), &mut opener.task)
            .await
            .unwrap()
            .unwrap();
        assert!(opener.is_finished());
        let text = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("external opener stdout"));
        assert!(text.contains("external opener stderr"));
        assert!(text.contains("External opener failed") && text.contains("7"));
        assert!(!text.contains('\x00'));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_opener_retires_its_launcher() {
        let root = tempfile::tempdir().unwrap();
        let pid_file = root.path().join("pid");
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "echo $$ > \"$1\"; exec sleep 30", "fixture"])
            .arg(&pid_file);
        let opener = ExternalOpener::spawn(command).unwrap();
        let pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(pid) = tokio::fs::read_to_string(&pid_file).await {
                    if !pid.trim().is_empty() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        drop(opener);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = tokio::process::Command::new("/bin/kill")
                    .args(["-0", pid.trim()])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await
                    .unwrap();
                if !status.success() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("opener launcher must be reaped on owner drop");
    }

    #[test]
    fn tail_is_bounded_and_counts_discarded_bytes() {
        let mut tail = Tail::default();
        tail.push(&vec![b'a'; 100_000]);
        tail.push(b"latest");
        assert_eq!(tail.bytes.len(), RETAIN_BYTES);
        assert_eq!(tail.omitted, 100_006 - RETAIN_BYTES as u64);
        assert!(tail.bytes.make_contiguous().ends_with(b"latest"));
    }

    #[tokio::test]
    async fn finish_joins_reader_without_descendant_eof() {
        let (mut writer, reader) = tokio::io::duplex(128);
        let mut diagnostics = DiagnosticReader::new(reader, "fixture");
        writer.write_all(b"partial line").await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), diagnostics.finish())
            .await
            .unwrap()
            .unwrap();
        assert!(diagnostics.task.is_none());
        assert!(writer.write_all(b"closed").await.is_err());
        diagnostics.finish().await.unwrap();
    }

    #[tokio::test]
    async fn drop_aborts_quiet_reader() {
        let (mut writer, reader) = tokio::io::duplex(128);
        let diagnostics = DiagnosticReader::new(reader, "fixture");
        let aborted = diagnostics.task.as_ref().unwrap().abort_handle();
        drop(diagnostics);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !aborted.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(writer.write_all(b"closed").await.is_err());
    }
}
