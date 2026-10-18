//! Real dispatcher loop ownership tests without runtime startup or providers.
use super::*;
use crate::test_support::server;
use agent_client_protocol_schema as acp;
use connection::Inbound;
use serde_json::json;
use std::io::Write;
use std::pin::Pin;
use std::sync::Mutex as StdMutex;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::AsyncWrite;
use tokio::sync::{oneshot, Notify};

#[derive(Clone)]
struct LogWriter(Arc<StdMutex<Vec<u8>>>);
impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run<F: std::future::Future>(future: F) -> (F::Output, String) {
    let logs = LogWriter(Arc::new(StdMutex::new(Vec::new())));
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let output = tracing::subscriber::with_default(subscriber, || runtime.block_on(future));
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    (output, text)
}

fn request() -> Inbound {
    Inbound::Request {
        id: acp::RequestId::Number(7),
        method: "session/load".into(),
        params: None,
    }
}

struct PanicPayload {
    dropped: Option<oneshot::Sender<()>>,
    _private: &'static str,
}
impl Drop for PanicPayload {
    fn drop(&mut self) {
        if let Some(sender) = self.dropped.take() {
            let _ = sender.send(());
        }
    }
}
struct PanicWriter(Option<PanicPayload>);
impl AsyncWrite for PanicWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.0.take() {
            Some(payload) => std::panic::panic_any(payload),
            None => panic!("private_dispatch_string_marker"),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[test]
fn completed_panic_is_reaped_and_reported_before_eof() {
    let ((reaped_before_eof, still_open), logs) = run(async {
        let (dropped, receiver) = oneshot::channel();
        let server = server(Connection::new(PanicWriter(Some(PanicPayload {
            dropped: Some(dropped),
            _private: "private_dispatch_panic_marker",
        }))));
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(dispatch_messages(server, rx));
        tx.send(request()).unwrap();
        let observed = matches!(
            tokio::time::timeout(Duration::from_secs(2), receiver).await,
            Ok(Ok(()))
        );
        let still_open = !task.is_finished();
        // Close and drain even when baseline retained the completed handle.
        drop(tx);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        (observed, still_open)
    });
    assert!(still_open, "dispatch loop returned before inbound EOF");
    assert!(
        reaped_before_eof,
        "completed panic payload retained until EOF; logs: {logs}"
    );
    assert!(logs.contains("ACP: dispatch task failed"), "{logs}");
    assert!(logs.contains("panicked=true"), "{logs}");
    assert!(logs.contains("cancelled=false"), "{logs}");
    assert!(!logs.contains("private_dispatch_panic_marker"), "{logs}");
}

#[derive(Default)]
struct Gate {
    released: bool,
    waker: Option<std::task::Waker>,
    bytes: Vec<u8>,
}
struct GateWriter {
    gate: Arc<StdMutex<Gate>>,
    entered: Arc<Notify>,
}
impl AsyncWrite for GateWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut gate = self.gate.lock().unwrap();
        if !gate.released {
            gate.waker = Some(cx.waker().clone());
            self.entered.notify_one();
            return Poll::Pending;
        }
        gate.bytes.extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[test]
fn blocked_response_keeps_cancel_responsive_and_eof_drains() {
    let ((entered, cancelled, waited, bytes), logs) = run(async {
        let gate = Arc::new(StdMutex::new(Gate::default()));
        let entered = Arc::new(Notify::new());
        let server = server(Connection::new(GateWriter {
            gate: gate.clone(),
            entered: entered.clone(),
        }));
        let session =
            sessions::SessionState::new(acp::SessionId::new("fixture"), "/unused-fixture".into());
        let token = session.cancel_token.lock().clone();
        server.sessions.insert(session);
        let (tx, rx) = mpsc::unbounded_channel();
        let mut task = tokio::spawn(dispatch_messages(server, rx));
        tx.send(request()).unwrap();
        let did_enter = tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .is_ok();
        tx.send(Inbound::Notification {
            method: "session/cancel".into(),
            params: Some(json!({"sessionId":"fixture"})),
        })
        .unwrap();
        let cancelled = tokio::time::timeout(Duration::from_secs(2), token.cancelled())
            .await
            .is_ok();
        drop(tx);
        let waited = tokio::time::timeout(Duration::from_millis(50), &mut task)
            .await
            .is_err();
        {
            let mut state = gate.lock().unwrap();
            state.released = true;
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }
        if waited {
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap();
        }
        let bytes = gate.lock().unwrap().bytes.clone();
        (did_enter, cancelled, waited, bytes)
    });
    assert!(entered && cancelled && waited);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        json!({"jsonrpc":"2.0","id":7,"error":acp::Error::method_not_found()})
    );
    assert!(!logs.contains("WARN"), "{logs}");
}

#[test]
fn empty_and_successful_dispatch_are_quiet() {
    let (_, logs) = run(async {
        for with_request in [false, true] {
            let gate = Arc::new(StdMutex::new(Gate {
                released: true,
                ..Default::default()
            }));
            let server = server(Connection::new(GateWriter {
                gate: gate.clone(),
                entered: Arc::new(Notify::new()),
            }));
            let (tx, rx) = mpsc::unbounded_channel();
            if with_request {
                tx.send(request()).unwrap();
            }
            drop(tx);
            tokio::time::timeout(Duration::from_secs(2), dispatch_messages(server, rx))
                .await
                .unwrap();
            let bytes = gate.lock().unwrap().bytes.clone();
            if with_request {
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                    json!({"jsonrpc":"2.0","id":7,"error":acp::Error::method_not_found()})
                );
            } else {
                assert!(bytes.is_empty());
            }
        }
    });
    assert!(!logs.contains("WARN"), "{logs}");
}

#[test]
fn panic_string_is_not_logged() {
    let (_, logs) = run(async {
        let server = server(Connection::new(PanicWriter(None)));
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(request()).unwrap();
        drop(tx);
        tokio::time::timeout(Duration::from_secs(2), dispatch_messages(server, rx))
            .await
            .unwrap();
    });
    assert!(logs.contains("ACP: dispatch task failed"), "{logs}");
    assert!(logs.contains("panicked=true"), "{logs}");
    assert!(!logs.contains("private_dispatch_string_marker"), "{logs}");
}
