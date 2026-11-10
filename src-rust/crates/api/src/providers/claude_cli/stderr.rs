//! Turn-owned stderr draining. Keep only a byte-bounded tail, including while
//! a pipe remains open, and acknowledge EOF or cancellation before collection.

use std::{io::ErrorKind, sync::Arc, time::Duration};

use parking_lot::Mutex;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    task::JoinSet,
};

const TAIL_BYTES: usize = 16 * 1024;
// A descendant may retain the pipe after stdout/child completion. This is a
// drain deadline, not a sleep used as evidence that the reader has completed.
const DRAIN_DEADLINE: Duration = Duration::from_millis(250);

enum Completion {
    Reading,
    Eof,
    ReadError(ErrorKind),
    Cancelled,
    TaskFailed,
}

struct DiagnosticTail {
    bytes: Vec<u8>,
    truncated: bool,
    completion: Completion,
}

impl DiagnosticTail {
    fn append(&mut self, bytes: &[u8]) {
        let overflow = (self.bytes.len() + bytes.len()).saturating_sub(TAIL_BYTES);
        self.truncated |= overflow > 0;
        if bytes.len() >= TAIL_BYTES {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&bytes[bytes.len() - TAIL_BYTES..]);
        } else {
            if overflow > 0 {
                self.bytes.copy_within(overflow.., 0);
                self.bytes.truncate(self.bytes.len() - overflow);
            }
            self.bytes.extend_from_slice(bytes);
        }
    }

    fn snapshot(&self) -> String {
        // Decode only the retained bytes. Lossy decoding also preserves useful
        // output when the tail starts inside UTF-8 or the CLI emits binary data.
        let text = String::from_utf8_lossy(&self.bytes);
        let mut detail = String::new();
        if self.truncated {
            detail.push_str("[stderr truncated; showing last 16384 bytes]\n");
        }
        detail.push_str(text.trim());
        let status = match self.completion {
            Completion::ReadError(kind) => Some(format!("[stderr read failed: {kind}]")),
            Completion::Cancelled => Some("[stderr collection stopped before EOF]".into()),
            Completion::TaskFailed => Some("[stderr reader task failed]".into()),
            Completion::Reading | Completion::Eof => None,
        };
        if let Some(status) = status {
            if !detail.is_empty() {
                detail.push('\n');
            }
            detail.push_str(&status);
        }
        detail
    }
}

pub(super) struct StderrDrain {
    tail: Arc<Mutex<DiagnosticTail>>,
    // JoinSet aborts the reader on drop, including a never-polled stream or
    // cancelled startup/finish future. There is no detached cleanup task.
    tasks: JoinSet<()>,
}

impl StderrDrain {
    pub(super) fn new<R: AsyncRead + Unpin + Send + 'static>(reader: Option<R>) -> Self {
        let tail = Arc::new(Mutex::new(DiagnosticTail {
            bytes: Vec::with_capacity(TAIL_BYTES),
            truncated: false,
            completion: Completion::Eof,
        }));
        let mut tasks = JoinSet::new();
        if let Some(mut reader) = reader {
            tail.lock().completion = Completion::Reading;
            let sink = Arc::clone(&tail);
            tasks.spawn(async move {
                let mut chunk = [0_u8; 4096];
                loop {
                    match reader.read(&mut chunk).await {
                        Ok(0) => {
                            sink.lock().completion = Completion::Eof;
                            return;
                        }
                        Ok(count) => sink.lock().append(&chunk[..count]),
                        Err(error) => {
                            sink.lock().completion = Completion::ReadError(error.kind());
                            return;
                        }
                    }
                }
            });
        }
        Self { tail, tasks }
    }

    pub(super) async fn finish(&mut self) -> String {
        match tokio::time::timeout(DRAIN_DEADLINE, self.tasks.join_next()).await {
            Ok(Some(Err(_))) => self.tail.lock().completion = Completion::TaskFailed,
            Err(_) => {
                // Await the abort acknowledgement so the pipe and reader buffer
                // have actually been released before returning diagnostics.
                self.tasks.shutdown().await;
                let mut tail = self.tail.lock();
                if matches!(tail.completion, Completion::Reading) {
                    tail.completion = Completion::Cancelled;
                }
            }
            Ok(_) => {}
        }
        self.tail.lock().snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::io::{AsyncWriteExt, ReadBuf};

    // A deterministic read failure after a successful partial read, which real
    // process pipes cannot portably induce without losing the preceding bytes.
    struct FailingReader(bool);
    impl AsyncRead for FailingReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.0 {
                Poll::Ready(Err(std::io::Error::from(ErrorKind::BrokenPipe)))
            } else {
                self.0 = true;
                buf.put_slice(b"partial failure diagnostic");
                Poll::Ready(Ok(()))
            }
        }
    }

    #[tokio::test]
    async fn partial_read_error_preserves_output_and_acknowledges_completion() {
        let mut drain = StderrDrain::new(Some(FailingReader(false)));
        let detail = drain.finish().await;
        assert!(detail.contains("partial failure diagnostic"));
        assert!(detail.contains("stderr read failed"));
        assert!(matches!(
            drain.tail.lock().completion,
            Completion::ReadError(ErrorKind::BrokenPipe)
        ));
        assert!(drain.tasks.is_empty());
    }

    #[tokio::test]
    async fn eof_completion_does_not_wait_for_deadline() {
        let mut drain = StderrDrain::new(Some(&b"  complete diagnostic\n"[..]));
        assert_eq!(drain.finish().await, "complete diagnostic");
        assert!(matches!(drain.tail.lock().completion, Completion::Eof));
        assert!(drain.tasks.is_empty());
    }

    #[tokio::test]
    async fn live_pipe_keeps_memory_bounded_and_finish_releases_reader() {
        let (mut writer, reader) = tokio::io::duplex(4096);
        let mut drain = StderrDrain::new(Some(reader));
        let chunk = [b'x'; 4096];
        for _ in 0..1024 {
            writer.write_all(&chunk).await.unwrap();
            let tail = drain.tail.lock();
            assert!(tail.bytes.len() <= TAIL_BYTES);
            assert_eq!(tail.bytes.capacity(), TAIL_BYTES);
        }
        writer.write_all(b"final diagnostic").await.unwrap();
        let detail = drain.finish().await;
        assert!(detail.contains("truncated"));
        assert!(detail.contains("final diagnostic"));
        assert!(detail.contains("stopped before EOF"));
        assert!(drain.tasks.is_empty());
        assert_eq!(
            writer.write_all(b"closed").await.unwrap_err().kind(),
            ErrorKind::BrokenPipe
        );
    }

    #[tokio::test]
    async fn utf8_split_across_reads_is_decoded_after_collection() {
        let (mut writer, reader) = tokio::io::duplex(1);
        let mut drain = StderrDrain::new(Some(reader));
        for byte in "diagnostic: λ終".as_bytes() {
            writer.write_all(&[*byte]).await.unwrap();
        }
        drop(writer);
        assert_eq!(drain.finish().await, "diagnostic: λ終");
    }

    #[tokio::test]
    async fn dropping_during_collection_aborts_the_owned_reader() {
        let (mut writer, reader) = tokio::io::duplex(1);
        let mut drain = StderrDrain::new(Some(reader));
        {
            let finish = drain.finish();
            tokio::pin!(finish);
            tokio::select! {
                detail = &mut finish => panic!("open pipe unexpectedly completed: {detail}"),
                _ = tokio::task::yield_now() => {}
            }
        }
        drop(drain);
        tokio::time::timeout(Duration::from_secs(1), async {
            // Fill the duplex buffer if the task has not observed its abort yet.
            // The second write completes only when the reader closes the pipe.
            loop {
                if writer.write_all(b"x").await.is_err() {
                    break;
                }
            }
        })
        .await
        .expect("reader survived cancellation of collection");
    }

    #[test]
    fn tail_handles_oversized_chunks_and_invalid_utf8_with_bounded_storage() {
        let mut tail = DiagnosticTail {
            bytes: Vec::with_capacity(TAIL_BYTES),
            truncated: false,
            completion: Completion::Eof,
        };
        tail.append(&vec![b'x'; TAIL_BYTES * 2]);
        tail.append(b"\xff final diagnostic");
        assert_eq!(tail.bytes.len(), TAIL_BYTES);
        assert_eq!(tail.bytes.capacity(), TAIL_BYTES);
        assert!(tail.snapshot().contains("� final diagnostic"));
    }
}
