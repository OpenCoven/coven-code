//! Actual reader and dispatch orchestration with no runtime startup or network.
use super::*;
use agent_client_protocol_schema as acp;
use claurst_core::{Config, Settings};
use serde_json::{json, Value};
use std::pin::Pin;
use std::sync::Mutex as StdMutex;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Notify;

const REQUEST: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"session/load\"}\n";

fn server(connection: Arc<Connection>) -> Arc<AgentServer> {
    let settings = Settings::default();
    let config = Config::default();
    let permission_manager = Arc::new(StdMutex::new(
        claurst_core::permissions::PermissionManager::new(
            config.permission_mode.clone(),
            &settings,
        ),
    ));
    let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
        api_key: "unused-fixture-key".into(),
        api_base: "http://127.0.0.1:9".into(),
        ..Default::default()
    })
    .unwrap();
    AgentServer::new(
        connection,
        Arc::new(AgentRuntime {
            config,
            settings,
            api_client: Arc::new(client),
            provider_registry: Arc::new(claurst_api::ProviderRegistry::new()),
            tools: Arc::new(Vec::new()),
            cost_tracker: claurst_core::CostTracker::new(),
            query_config: claurst_query::QueryConfig::default(),
            mcp_manager: None,
            permission_manager,
            working_dir: std::path::PathBuf::from("/unused-fixture"),
        }),
    )
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

struct FailingReader {
    prefix: &'static [u8],
    failed: Arc<Notify>,
}
impl AsyncRead for FailingReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if !self.prefix.is_empty() {
            let count = buf.remaining().min(self.prefix.len());
            buf.put_slice(&self.prefix[..count]);
            self.prefix = &self.prefix[count..];
            return Poll::Ready(Ok(()));
        }
        self.failed.notify_one();
        Poll::Ready(Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "fixture transport reset",
        )))
    }
}

fn assert_transport_error(result: anyhow::Result<()>, kind: std::io::ErrorKind) {
    let error = result.expect_err("reader transport failure must reach caller");
    assert_eq!(error.to_string(), "ACP reader loop failed");
    assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().kind(), kind);
}

#[tokio::test]
async fn invalid_utf8_reaches_caller() {
    let server = server(Connection::new(tokio::io::sink()));
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        run_connection(server, &b"\xff\n"[..]),
    )
    .await
    .unwrap();
    assert_transport_error(result, std::io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn reader_io_failure_reaches_caller() {
    let server = server(Connection::new(tokio::io::sink()));
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        run_connection(
            server,
            FailingReader {
                prefix: b"",
                failed: Arc::new(Notify::new()),
            },
        ),
    )
    .await
    .unwrap();
    assert_transport_error(result, std::io::ErrorKind::ConnectionReset);
}

#[tokio::test]
async fn empty_eof_succeeds_without_output() {
    let gate = Arc::new(StdMutex::new(Gate {
        released: true,
        ..Default::default()
    }));
    let server = server(Connection::new(GateWriter {
        gate: gate.clone(),
        entered: Arc::new(Notify::new()),
    }));
    tokio::time::timeout(
        Duration::from_secs(2),
        run_connection(server, tokio::io::empty()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(gate.lock().unwrap().bytes.is_empty());
}

#[tokio::test]
async fn malformed_json_responds_and_continues_until_eof() {
    let gate = Arc::new(StdMutex::new(Gate {
        released: true,
        ..Default::default()
    }));
    let server = server(Connection::new(GateWriter {
        gate: gate.clone(),
        entered: Arc::new(Notify::new()),
    }));
    let mut input = b"{not json}\n".to_vec();
    input.extend_from_slice(REQUEST);
    tokio::time::timeout(
        Duration::from_secs(2),
        run_connection(server, input.as_slice()),
    )
    .await
    .unwrap()
    .unwrap();
    let bytes = gate.lock().unwrap().bytes.clone();
    assert!(bytes.ends_with(b"\n"));
    let responses: Vec<Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        responses,
        vec![
            json!({"jsonrpc":"2.0","id":null,"error":acp::Error::parse_error()}),
            json!({"jsonrpc":"2.0","id":7,"error":acp::Error::method_not_found()}),
        ]
    );
}

#[tokio::test]
async fn reader_error_waits_for_blocked_dispatch_then_propagates() {
    let gate = Arc::new(StdMutex::new(Gate::default()));
    let entered = Arc::new(Notify::new());
    let failed = Arc::new(Notify::new());
    let server = server(Connection::new(GateWriter {
        gate: gate.clone(),
        entered: entered.clone(),
    }));
    let mut task = tokio::spawn(run_connection(
        server,
        FailingReader {
            prefix: REQUEST,
            failed: failed.clone(),
        },
    ));
    let entered = tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .is_ok();
    let failed = tokio::time::timeout(Duration::from_secs(2), failed.notified())
        .await
        .is_ok();
    let premature = tokio::time::timeout(Duration::from_millis(50), &mut task).await;
    let waited = premature.is_err();
    // Always release the writer before asserting, including on baseline failure.
    {
        let mut state = gate.lock().unwrap();
        state.released = true;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
    let result = match premature {
        Ok(result) => result.unwrap(),
        Err(_) => tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
    };
    assert!(entered && failed && waited);
    let bytes = gate.lock().unwrap().bytes.clone();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"jsonrpc":"2.0","id":7,"error":acp::Error::method_not_found()})
    );
    assert_transport_error(result, std::io::ErrorKind::ConnectionReset);
}

#[tokio::test]
async fn saturated_handlers_reject_excess_and_route_cancel_before_output_unblocks() {
    use tokio::io::AsyncWriteExt;
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
    let (mut input, reader) = tokio::io::duplex(16384);
    let task = tokio::spawn(run_connection(server, reader));
    for id in 0..64 {
        input
            .write_all(format!("{{\"id\":{id},\"method\":\"session/load\"}}\n").as_bytes())
            .await
            .unwrap();
    }
    input
        .write_all(b"{\"method\":\"session/cancel\",\"params\":{\"sessionId\":\"fixture\"}}\n")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), token.cancelled())
        .await
        .expect("cancel must bypass both saturated handlers and blocked output");
    assert!(!task.is_finished());
    {
        let mut state = gate.lock().unwrap();
        state.released = true;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
    drop(input);
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let bytes = gate.lock().unwrap().bytes.clone();
    let responses: Vec<Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 64);
    assert_eq!(
        responses
            .iter()
            .filter(|r| r["error"]["code"] == -32601)
            .count(),
        32
    );
    assert_eq!(
        responses
            .iter()
            .filter(|r| r["error"]["code"] == -32000)
            .count(),
        32
    );
    let mut ids: Vec<i64> = responses
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, (0..64).collect::<Vec<_>>());
}

#[tokio::test]
async fn eof_reaps_handlers_when_client_never_reads() {
    let (_unread, writer) = tokio::io::duplex(1);
    let server = server(Connection::new(writer));
    let result = tokio::time::timeout(
        Duration::from_secs(4),
        run_connection(server.clone(), REQUEST),
    )
    .await
    .expect("bounded shutdown must retire blocked writers");
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("ACP reader loop failed"));
    assert_eq!(
        Arc::strong_count(&server),
        1,
        "dispatcher retained server after shutdown"
    );
}

#[tokio::test]
async fn oversized_unterminated_input_is_rejected_before_eof() {
    use tokio::io::AsyncWriteExt;
    let server = server(Connection::new(tokio::io::sink()));
    let (mut input, reader) = tokio::io::duplex(4096);
    let task = tokio::spawn(run_connection(server, reader));
    let oversized = vec![b'x'; 8 * 1024 * 1024 + 1];
    input.write_all(&oversized).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    let error = result.unwrap_err();
    assert!(format!("{error:#}").contains("frame exceeds 8 MiB"));
    drop(input);
}
