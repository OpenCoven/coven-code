//! Bidirectional newline-delimited JSON-RPC 2.0 connection over arbitrary
//! `AsyncRead` / `AsyncWrite` half-duplex pair (typically stdin/stdout).
//!
//! Both sides can send requests, responses, and notifications. Outbound
//! requests are tracked in a pending map so their responses can be routed
//! back to the awaiting caller.
//!
//! The wire format matches the Agent Client Protocol: each message is a
//! single UTF-8 line terminated by `\n` carrying a JSON-RPC 2.0 envelope.

use std::io::Cursor;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use agent_client_protocol_schema as acp;
use dashmap::DashMap;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, trace, warn};

const JSONRPC_VERSION: &str = "2.0";

/// One parsed inbound JSON-RPC message — request, notification, or response.
///
/// Responses are NOT included here because the connection routes them
/// internally to the awaiting `send_request` future.
#[derive(Debug)]
pub enum Inbound {
    Request {
        id: acp::RequestId,
        method: String,
        params: Option<Value>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
}

/// Bidirectional connection over newline-delimited JSON-RPC.
///
/// Clone the returned `Arc` to share a sender. Drive exactly one reader with
/// [`run_reader`]; its exit closes the response path for outbound requests.
pub struct Connection {
    writer: Mutex<WriterState>,
    pending: DashMap<String, oneshot::Sender<Result<Value, acp::Error>>>,
    next_outbound_id: AtomicI64,
    reader_closed: CancellationToken,
}

// Retain a partially written frame until the next send or connection drop.
// Cancellation or an error cannot retract bytes already accepted by the writer;
// a later send may finish that frame even though its original caller has left.
struct WriterState {
    stream: Box<dyn AsyncWrite + Send + Unpin>,
    pending: Option<Cursor<Vec<u8>>>,
}

impl WriterState {
    async fn finish_pending(&mut self) -> std::io::Result<()> {
        if let Some(frame) = &mut self.pending {
            // Cursor advances on every successful write, including before cancellation.
            self.stream.write_all_buf(frame).await?;
            self.stream.flush().await?;
            trace!(bytes = frame.get_ref().len(), "ACP wire send");
        }
        self.pending = None;
        Ok(())
    }
}

/// Remove registration on every request exit, including future cancellation.
struct PendingRequest<'a> {
    connection: &'a Connection,
    id: String,
}

impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        self.connection.pending.remove(&self.id);
    }
}

impl Connection {
    /// Create a new connection over the given writer. The caller drives the
    /// reader by calling `run_reader` separately.
    pub fn new<W>(writer: W) -> Arc<Self>
    where
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Arc::new(Self {
            writer: Mutex::new(WriterState {
                stream: Box::new(writer),
                pending: None,
            }),
            pending: DashMap::new(),
            next_outbound_id: AtomicI64::new(1),
            reader_closed: CancellationToken::new(),
        })
    }

    /// Send a successful JSON-RPC response to a previously received request.
    pub async fn send_response<R: Serialize>(
        self: &Arc<Self>,
        id: acp::RequestId,
        result: R,
    ) -> anyhow::Result<()> {
        let value = serde_json::to_value(result)?;
        let msg = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "result": value,
        });
        self.write_line(&msg).await
    }

    /// Send a JSON-RPC error response.
    pub async fn send_error_response(
        self: &Arc<Self>,
        id: acp::RequestId,
        error: acp::Error,
    ) -> anyhow::Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "error": error,
        });
        self.write_line(&msg).await
    }

    /// Send a notification (no id, no response expected).
    pub async fn send_notification<P: Serialize>(
        self: &Arc<Self>,
        method: &str,
        params: P,
    ) -> anyhow::Result<()> {
        let value = serde_json::to_value(params)?;
        let msg = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": method,
            "params": value,
        });
        self.write_line(&msg).await
    }

    /// Send an outbound request and await the response.
    ///
    /// The caller is blocked until the matching response arrives via the
    /// reader loop, or the connection is closed (in which case `Err(...)`).
    pub async fn send_request<P: Serialize, R: DeserializeOwned>(
        self: &Arc<Self>,
        method: &str,
        params: P,
    ) -> anyhow::Result<Result<R, acp::Error>> {
        if self.reader_closed.is_cancelled() {
            anyhow::bail!("connection closed before sending request '{}'", method);
        }
        let params_value = serde_json::to_value(params)?;
        let raw_id = self.next_outbound_id.fetch_add(1, Ordering::Relaxed);
        let id_key = format!("n:{}", raw_id);
        let (tx, mut rx) = oneshot::channel();
        self.pending.insert(id_key.clone(), tx);
        let _pending = PendingRequest {
            connection: self,
            id: id_key,
        };

        let msg = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": raw_id,
            "method": method,
            "params": params_value,
        });
        let write = self.write_line(&msg);
        tokio::pin!(write);
        let mut written = false;
        let response = loop {
            tokio::select! {
                biased;
                // A delivered reply wins over simultaneous EOF, even if the
                // writer is still flushing. Reader exit also interrupts writes
                // blocked on backpressure or the writer mutex.
                response = &mut rx => break response.ok(),
                result = &mut write, if !written => {
                    result?;
                    written = true;
                }
                _ = self.reader_closed.cancelled() => break None,
            }
        };

        match response {
            Some(Ok(value)) => {
                let typed: R = serde_json::from_value(value)?;
                Ok(Ok(typed))
            }
            Some(Err(err)) => Ok(Err(err)),
            None => Err(anyhow::anyhow!(
                "connection closed while awaiting response to '{}'",
                method
            )),
        }
    }

    async fn write_line(self: &Arc<Self>, value: &Value) -> anyhow::Result<()> {
        let mut buf = serde_json::to_vec(value)?;
        buf.push(b'\n');
        let mut w = self.writer.lock().await;
        // A canceled send that never wrote any bytes can be discarded safely.
        if w.pending
            .as_ref()
            .is_some_and(|frame| frame.position() == 0)
        {
            w.pending = None;
        }
        w.finish_pending().await?;
        w.pending = Some(Cursor::new(buf));
        w.finish_pending().await?;
        Ok(())
    }

    /// Look up and resolve a pending outbound request when a response arrives.
    fn complete_pending(&self, id: &acp::RequestId, payload: Result<Value, acp::Error>) {
        let key = id_to_key(id);
        if let Some((_, tx)) = self.pending.remove(&key) {
            let _ = tx.send(payload);
        } else {
            let (id_kind, id_bytes) = match id {
                acp::RequestId::Null => ("null", 0),
                // id_to_key prefixes the formatted numeric ID with "n:".
                acp::RequestId::Number(_) => ("number", key.len() - 2),
                acp::RequestId::Str(value) => ("string", value.len()),
            };
            warn!(
                id_kind,
                id_bytes, "ACP: received response for unknown request id"
            );
        }
    }
}

fn id_to_key(id: &acp::RequestId) -> String {
    match id {
        acp::RequestId::Null => "null".to_string(),
        acp::RequestId::Number(n) => format!("n:{}", n),
        acp::RequestId::Str(s) => format!("s:{}", s),
    }
}

/// Spawn the reader loop. Inbound requests and notifications are forwarded
/// to `tx`. Responses are matched against `connection.pending` internally.
/// The future completes when stdin reaches EOF.
pub fn run_reader<R>(
    connection: Arc<Connection>,
    reader: R,
    tx: mpsc::UnboundedSender<Inbound>,
) -> impl std::future::Future<Output = anyhow::Result<()>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    // Create the guard before the first poll so dropping an unpolled reader
    // future also closes the response path.
    let close_on_exit = connection.reader_closed.clone().drop_guard();
    async move {
        let _close_on_exit = close_on_exit;
        let mut buffered = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            let n = buffered.read_line(&mut line).await?;
            if n == 0 {
                debug!("ACP: stdin EOF, shutting down reader");
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    // Spec says: if the message can't be parsed, send a parse-error
                    // response with id=null. Best-effort here since we may not even
                    // know if it was a request or notification.
                    error!(error = %e, "ACP: parse error");
                    let _ = connection
                        .send_error_response(acp::RequestId::Null, acp::Error::parse_error())
                        .await;
                    continue;
                }
            };

            // Distinguish response (has id + (result|error)) from request (has id + method)
            // from notification (no id, has method).
            let has_id = v.get("id").is_some();
            let has_method = v.get("method").is_some();
            let has_result = v.get("result").is_some();
            let has_error = v.get("error").is_some();

            if has_id && (has_result || has_error) && !has_method {
                // Response — route to pending.
                let id: acp::RequestId =
                    serde_json::from_value(v["id"].clone()).unwrap_or(acp::RequestId::Null);
                if has_result {
                    let value = v["result"].clone();
                    connection.complete_pending(&id, Ok(value));
                } else {
                    let err: acp::Error = match serde_json::from_value(v["error"].clone()) {
                        Ok(e) => e,
                        Err(_) => acp::Error::internal_error().data(Some(v["error"].clone())),
                    };
                    connection.complete_pending(&id, Err(err));
                }
            } else if has_id && has_method {
                // Request.
                let id: acp::RequestId =
                    serde_json::from_value(v["id"].clone()).unwrap_or(acp::RequestId::Null);
                let method = v
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let params = v.get("params").cloned();
                if tx.send(Inbound::Request { id, method, params }).is_err() {
                    break;
                }
            } else if has_method {
                // Notification.
                let method = v
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let params = v.get("params").cloned();
                if tx.send(Inbound::Notification { method, params }).is_err() {
                    break;
                }
            } else {
                let json_type = match &v {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                };
                warn!(
                    json_type,
                    bytes = trimmed.len(),
                    has_id,
                    has_method,
                    has_result,
                    has_error,
                    "ACP: unrecognized JSON-RPC message shape"
                );
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncReadExt};

    #[tokio::test]
    async fn dropping_unpolled_reader_closes_requests() {
        let connection = Connection::new(tokio::io::sink());
        let (tx, _rx) = mpsc::unbounded_channel();
        drop(run_reader(connection.clone(), tokio::io::empty(), tx));
        assert!(tokio::time::timeout(
            std::time::Duration::from_secs(2),
            connection.send_request::<_, Value>("test", ()),
        )
        .await
        .expect("unpolled reader drop must close requests")
        .is_err());
        assert!(connection.pending.is_empty());
    }

    #[tokio::test]
    async fn write_failure_removes_pending_entry() {
        let (client, writer) = duplex(1);
        drop(client);
        let connection = Connection::new(writer);
        assert!(connection
            .send_request::<_, Value>("test", ())
            .await
            .is_err());
        assert!(connection.pending.is_empty());
    }

    #[tokio::test]
    async fn cancelling_blocked_write_removes_pending_entry() {
        let (_client, writer) = duplex(1);
        let connection = Connection::new(writer);
        let mut request = Box::pin(connection.send_request::<_, Value>("test", ()));
        tokio::select! {
            biased;
            _ = &mut request => panic!("write should be blocked"),
            _ = std::future::ready(()) => {}
        }
        assert_eq!(connection.pending.len(), 1);
        drop(request);
        assert!(connection.pending.is_empty());
    }

    #[tokio::test]
    async fn delivered_response_wins_over_eof_while_flush_is_blocked() {
        use std::pin::Pin;
        use std::task::{Context, Poll};

        struct BlockedFlush;
        impl AsyncWrite for BlockedFlush {
            fn poll_write(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
                data: &[u8],
            ) -> Poll<std::io::Result<usize>> {
                Poll::Ready(Ok(data.len()))
            }
            fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
                Poll::Pending
            }
            fn poll_shutdown(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        let connection = Connection::new(BlockedFlush);
        let request = connection.send_request::<_, Value>("test", ());
        tokio::pin!(request);
        tokio::select! {
            biased;
            _ = &mut request => panic!("flush should be blocked"),
            _ = std::future::ready(()) => {}
        }
        let (tx, _rx) = mpsc::unbounded_channel();
        let response = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n";
        run_reader(connection.clone(), response.as_slice(), tx)
            .await
            .unwrap();
        let value = tokio::time::timeout(std::time::Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(value["ok"], true);
        assert!(connection.pending.is_empty());
    }

    #[tokio::test]
    async fn reader_exit_releases_pending_requests() {
        for mode in ["eof", "read_error", "abort"] {
            let (mut client_input, server_input) = duplex(8192);
            let (client_output, server_output) = duplex(8192);
            let connection = Connection::new(server_output);
            let (tx, _rx) = mpsc::unbounded_channel();
            let reader = tokio::spawn(run_reader(connection.clone(), server_input, tx));
            let request_connection = connection.clone();
            let request = tokio::spawn(async move {
                request_connection
                    .send_request::<_, Value>("session/request_permission", ())
                    .await
            });
            let mut client_output = BufReader::new(client_output);
            let mut line = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client_output.read_line(&mut line),
            )
            .await
            .unwrap()
            .unwrap();

            if mode == "read_error" {
                client_input.write_all(b"\xff\n").await.unwrap();
            } else if mode == "abort" {
                reader.abort();
            }
            drop(client_input);
            let result = reader.await;
            match mode {
                "eof" => assert!(result.unwrap().is_ok()),
                "read_error" => assert!(result.unwrap().is_err()),
                _ => assert!(result.unwrap_err().is_cancelled()),
            }
            let error = tokio::time::timeout(std::time::Duration::from_secs(2), request)
                .await
                .expect("reader exit must wake pending requests")
                .unwrap()
                .unwrap_err();
            assert!(error.to_string().contains("session/request_permission"));
            assert!(connection.pending.is_empty());
        }
    }

    #[tokio::test]
    async fn requests_after_eof_fail_without_waiting_for_a_response() {
        let connection = Connection::new(tokio::io::sink());
        let (tx, _rx) = mpsc::unbounded_channel();
        run_reader(connection.clone(), tokio::io::empty(), tx)
            .await
            .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            connection.send_request::<_, Value>("fs/read_text_file", ()),
        )
        .await
        .expect("request after EOF must not hang");
        assert!(result.is_err());
        assert!(connection.pending.is_empty());
    }

    #[tokio::test]
    async fn cancelled_request_removes_pending_entry() {
        let (client_output, server_output) = duplex(8192);
        let connection = Connection::new(server_output);
        let request_connection = connection.clone();
        let request = tokio::spawn(async move {
            request_connection
                .send_request::<_, Value>("test", ())
                .await
        });
        let mut output = BufReader::new(client_output);
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            output.read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert!(connection.pending.is_empty());
    }

    #[tokio::test]
    async fn serialization_failure_does_not_leave_pending_entry() {
        let connection = Connection::new(tokio::io::sink());
        let params = std::collections::HashMap::from([(vec![1, 2], "invalid JSON key")]);
        assert!(connection
            .send_request::<_, Value>("test", params)
            .await
            .is_err());
        assert!(connection.pending.is_empty());
    }

    #[tokio::test]
    async fn reader_exit_interrupts_a_blocked_request_write() {
        let (_client_output, server_output) = duplex(1);
        let connection = Connection::new(server_output);
        let request = connection.send_request::<_, Value>("test", ());
        tokio::pin!(request);
        tokio::select! {
            biased;
            _ = &mut request => panic!("write should be blocked by the full pipe"),
            _ = std::future::ready(()) => {}
        }
        assert_eq!(connection.pending.len(), 1);
        let (tx, _rx) = mpsc::unbounded_channel();
        run_reader(connection.clone(), tokio::io::empty(), tx)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(2), request)
                .await
                .expect("EOF must interrupt blocked writes")
                .is_err()
        );
        assert!(connection.pending.is_empty());
    }

    /// `send_response` must emit a newline-delimited JSON object containing
    /// `jsonrpc: "2.0"`, the request id, and the result payload.
    #[tokio::test]
    async fn send_response_writes_newline_delimited_jsonrpc() {
        let (client_read, server_write) = duplex(8192);
        let connection = Connection::new(server_write);

        connection
            .send_response(acp::RequestId::Number(7), serde_json::json!({"ok": true}))
            .await
            .unwrap();
        drop(connection);

        let mut buf = Vec::new();
        let mut client_read = client_read;
        client_read.read_to_end(&mut buf).await.unwrap();
        let line = std::str::from_utf8(&buf).unwrap();
        assert!(line.ends_with('\n'), "response missing newline: {line:?}");
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 7);
        assert_eq!(parsed["result"]["ok"], true);
    }

    /// `send_notification` must omit `id` so the receiver knows not to reply.
    #[tokio::test]
    async fn notifications_have_no_id() {
        let (client_read, server_write) = duplex(8192);
        let connection = Connection::new(server_write);

        connection
            .send_notification("session/update", serde_json::json!({"sessionId": "s1"}))
            .await
            .unwrap();
        drop(connection);

        let mut buf = Vec::new();
        let mut client_read = client_read;
        client_read.read_to_end(&mut buf).await.unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert!(parsed.get("id").is_none(), "notification must not carry id");
        assert_eq!(parsed["method"], "session/update");
    }

    /// The reader must distinguish requests, notifications, and responses by
    /// shape and route each to the right callback path.
    #[tokio::test]
    async fn reader_routes_inbound_messages() {
        // Drive the reader from an in-memory pipe.
        let (writer_handle, server_reader) = duplex(8192);
        let (_to_client, client_writer) = duplex(8192);
        let connection = Connection::new(client_writer);
        let (tx, mut rx) = mpsc::unbounded_channel();

        let reader_handle = tokio::spawn(run_reader(connection.clone(), server_reader, tx));

        // Inject a request, a notification, and an unsolicited response.
        let mut writer_handle = writer_handle;
        writer_handle
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1}}\n",
            )
            .await
            .unwrap();
        writer_handle
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"method\":\"session/cancel\",\"params\":{\"sessionId\":\"s1\"}}\n",
            )
            .await
            .unwrap();
        writer_handle
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{\"orphan\":true}}\n")
            .await
            .unwrap();
        drop(writer_handle); // EOF the reader

        let first = rx.recv().await.expect("expected request");
        match first {
            Inbound::Request { id, method, .. } => {
                assert_eq!(method, "initialize");
                assert!(matches!(id, acp::RequestId::Number(1)));
            }
            other => panic!("expected Request, got {other:?}"),
        }
        let second = rx.recv().await.expect("expected notification");
        match second {
            Inbound::Notification { method, .. } => {
                assert_eq!(method, "session/cancel");
            }
            other => panic!("expected Notification, got {other:?}"),
        }
        // Orphan response is routed internally; nothing surfaces here.
        assert!(rx.recv().await.is_none(), "no further messages expected");
        let _ = reader_handle.await;
    }

    /// `send_request` resolves when a response with the matching id arrives.
    #[tokio::test]
    async fn send_request_resolves_on_matching_response() {
        let (client_to_server, server_reader) = duplex(8192);
        let (server_to_client_reader, server_to_client) = duplex(8192);
        let connection = Connection::new(server_to_client);
        let (tx, _rx) = mpsc::unbounded_channel();
        let reader_handle = tokio::spawn(run_reader(connection.clone(), server_reader, tx));

        // Background: as a fake client, read the outbound request and write a
        // matching response.
        let client_loop = tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut server_to_client_reader = server_to_client_reader;
            let mut byte = [0u8; 1];
            // Read up to one full line.
            loop {
                let n = server_to_client_reader.read(&mut byte).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            let outbound: serde_json::Value = serde_json::from_slice(buf.trim_ascii_end()).unwrap();
            let id = outbound["id"].clone();
            // Send the response back through the client_to_server pipe.
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"echo": outbound["method"].as_str().unwrap()}
            });
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            let mut client_to_server = client_to_server;
            client_to_server.write_all(&bytes).await.unwrap();
            drop(client_to_server);
        });

        let response: Result<serde_json::Value, acp::Error> = connection
            .send_request("fs/read_text_file", serde_json::json!({"path": "/x"}))
            .await
            .unwrap();
        let response = response.unwrap();
        assert_eq!(response["echo"], "fs/read_text_file");
        let _ = client_loop.await;
        let _ = reader_handle.await;
    }
}

#[cfg(test)]
mod write_frame_tests {
    use super::*;
    use std::future::{poll_fn, Future};
    use std::io;
    use std::pin::Pin;
    use std::sync::Mutex as StdMutex;
    use std::task::{Context, Poll};
    use tokio::io::{duplex, AsyncReadExt};
    use tokio::time::{timeout, Duration};

    async fn poll_pending<F: Future>(future: Pin<&mut F>) {
        let mut future = future;
        poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }

    async fn send(connection: &Arc<Connection>, method: &str) -> anyhow::Result<()> {
        timeout(
            Duration::from_secs(2),
            connection.send_notification(method, ()),
        )
        .await
        .expect("notification write must finish within two seconds")
    }

    fn expected(methods: &[&str]) -> Vec<u8> {
        methods
            .iter()
            .flat_map(|method| {
                let mut bytes = serde_json::to_vec(&serde_json::json!({
                    "jsonrpc": "2.0", "method": method, "params": null
                }))
                .unwrap();
                bytes.push(b'\n');
                bytes
            })
            .collect()
    }

    #[tokio::test]
    async fn cancelled_partial_frame_finishes_before_next_frame() {
        let (mut reader, writer) = duplex(1);
        let connection = Connection::new(writer);
        let mut first = Box::pin(connection.send_notification("first", ()));
        poll_pending(first.as_mut()).await;
        let mut prefix = [0];
        reader.read_exact(&mut prefix).await.unwrap();
        assert_eq!(prefix, [b'{']);
        drop(first);
        let send = async move {
            send(&connection, "next").await.unwrap();
        };
        let read = async move {
            let mut bytes = prefix.to_vec();
            reader.read_to_end(&mut bytes).await.unwrap();
            bytes
        };
        let (_, bytes) = timeout(Duration::from_secs(2), async { tokio::join!(send, read) })
            .await
            .unwrap();
        assert_eq!(bytes, expected(&["first", "next"]));
    }

    #[derive(Default)]
    struct State {
        bytes: Vec<u8>,
        limit: Option<usize>,
        fail: bool,
        flush_blocked: bool,
        flush_polls: usize,
    }

    struct ControlledWriter(Arc<StdMutex<State>>);

    impl AsyncWrite for ControlledWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            let mut state = self.0.lock().unwrap();
            let available = state
                .limit
                .map_or(bytes.len(), |limit| limit.saturating_sub(state.bytes.len()));
            if available == 0 {
                return if state.fail {
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "fixture write failure",
                    )))
                } else {
                    Poll::Pending
                };
            }
            let count = available.min(bytes.len());
            state.bytes.extend_from_slice(&bytes[..count]);
            Poll::Ready(Ok(count))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            let mut state = self.0.lock().unwrap();
            state.flush_polls += 1;
            if state.flush_blocked {
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        }
        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.poll_flush(cx)
        }
    }

    #[tokio::test]
    async fn cancelled_unstarted_frame_is_not_sent_later() {
        let state = Arc::new(StdMutex::new(State {
            limit: Some(0),
            ..State::default()
        }));
        let connection = Connection::new(ControlledWriter(state.clone()));
        let mut first = Box::pin(connection.send_notification("cancelled", ()));
        poll_pending(first.as_mut()).await;
        drop(first);
        assert!(state.lock().unwrap().bytes.is_empty());
        state.lock().unwrap().limit = None;
        send(&connection, "next").await.unwrap();
        assert_eq!(state.lock().unwrap().bytes, expected(&["next"]));
    }

    #[tokio::test]
    async fn cancelled_mutex_waiter_does_not_replace_pending_frame() {
        let state = Arc::new(StdMutex::new(State {
            limit: Some(1),
            ..State::default()
        }));
        let connection = Connection::new(ControlledWriter(state.clone()));
        let mut first = Box::pin(connection.send_notification("first", ()));
        poll_pending(first.as_mut()).await;
        let mut waiter = Box::pin(connection.send_notification("cancelled", ()));
        poll_pending(waiter.as_mut()).await;
        drop(waiter);
        drop(first);
        state.lock().unwrap().limit = None;
        send(&connection, "next").await.unwrap();
        assert_eq!(state.lock().unwrap().bytes, expected(&["first", "next"]));
    }

    #[tokio::test]
    async fn cancelled_flush_is_retried_without_replaying_bytes() {
        let state = Arc::new(StdMutex::new(State {
            flush_blocked: true,
            ..State::default()
        }));
        let connection = Connection::new(ControlledWriter(state.clone()));
        let mut first = Box::pin(connection.send_notification("first", ()));
        poll_pending(first.as_mut()).await;
        drop(first);
        assert_eq!(state.lock().unwrap().bytes, expected(&["first"]));
        // A second cancelled send must still wait for the old flush, not write its own frame.
        let mut waiter = Box::pin(connection.send_notification("cancelled", ()));
        poll_pending(waiter.as_mut()).await;
        drop(waiter);
        assert_eq!(state.lock().unwrap().bytes, expected(&["first"]));
        state.lock().unwrap().flush_blocked = false;
        send(&connection, "next").await.unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.bytes, expected(&["first", "next"]));
        assert_eq!(state.flush_polls, 4);
    }

    #[tokio::test]
    async fn partial_write_error_preserves_offset_for_next_send() {
        let state = Arc::new(StdMutex::new(State {
            limit: Some(3),
            fail: true,
            ..State::default()
        }));
        let connection = Connection::new(ControlledWriter(state.clone()));
        let error = send(&connection, "first").await.unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(state.lock().unwrap().bytes.len(), 3);
        // Persistent errors propagate without writing any bytes from the new frame.
        assert!(send(&connection, "blocked").await.is_err());
        assert_eq!(state.lock().unwrap().bytes.len(), 3);
        state.lock().unwrap().limit = None;
        send(&connection, "next").await.unwrap();
        assert_eq!(state.lock().unwrap().bytes, expected(&["first", "next"]));
    }
}

#[cfg(test)]
mod log_payload_tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex as StdMutex;
    use tokio::io::AsyncReadExt;

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

    fn capture(input: &str) -> (String, Vec<Inbound>, Vec<u8>) {
        let logs = LogWriter(Arc::new(StdMutex::new(Vec::new())));
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (inbound, wire) = tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(async {
                let (mut output, writer) = tokio::io::duplex(8192);
                let connection = Connection::new(writer);
                let (tx, mut rx) = mpsc::unbounded_channel();
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    run_reader(connection.clone(), input.as_bytes(), tx),
                )
                .await
                .unwrap()
                .unwrap();
                let mut inbound = Vec::new();
                while let Ok(message) = rx.try_recv() {
                    inbound.push(message);
                }
                drop(connection);
                let mut wire = Vec::new();
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    output.read_to_end(&mut wire),
                )
                .await
                .unwrap()
                .unwrap();
                (inbound, wire)
            })
        });
        let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        (text, inbound, wire)
    }

    #[test]
    fn unrecognized_envelopes_log_shape_without_payload() {
        for (value, kind) in [
            (
                serde_json::json!({"private_key_marker": "private_value_marker", "params":{"prompt":"private_prompt_marker"}}),
                "object",
            ),
            (serde_json::json!(["private_value_marker"]), "array"),
            (serde_json::json!("private_value_marker"), "string"),
            (serde_json::json!(987654321), "number"),
            (serde_json::json!(true), "boolean"),
            (Value::Null, "null"),
            (serde_json::json!({"id":"private_id_marker"}), "object"),
        ] {
            let input = format!("{value}\n");
            let (logs, inbound, wire) = capture(&input);
            assert!(logs.contains("ACP: unrecognized JSON-RPC message shape"));
            assert!(
                !logs.contains("private_"),
                "inbound contents leaked: {logs}"
            );
            assert!(!logs.contains("987654321"));
            assert!(
                logs.contains(&format!("json_type=\"{kind}\"")),
                "missing type: {logs}"
            );
            assert!(logs.contains(&format!("bytes={}", input.trim().len())));
            assert!(logs.contains(&format!("has_id={}", value.get("id").is_some())));
            assert!(logs.contains("has_method=false"));
            assert!(logs.contains("has_result=false"));
            assert!(logs.contains("has_error=false"));
            assert!(inbound.is_empty());
            assert!(wire.is_empty());
        }
    }

    #[test]
    fn unknown_response_ids_log_kind_without_values() {
        for (id, kind, bytes) in [
            (serde_json::json!("private_id_marker"), "string", 17),
            (serde_json::json!("private_id_marker-é界"), "string", 23),
            (serde_json::json!(987654321), "number", 9),
            (Value::Null, "null", 0),
        ] {
            let input = format!(
                "{}\n",
                serde_json::json!({"id":id,"result":{"private_result_marker":true}})
            );
            let (logs, inbound, wire) = capture(&input);
            assert!(logs.contains("ACP: received response for unknown request id"));
            assert!(!logs.contains("private_"), "inbound id leaked: {logs}");
            assert!(!logs.contains("987654321"));
            assert!(logs.contains(&format!("id_kind=\"{kind}\"")));
            assert!(logs.contains(&format!("id_bytes={bytes}")));
            assert!(inbound.is_empty());
            assert!(wire.is_empty());
        }
    }

    #[test]
    fn valid_payloads_and_parse_error_wire_behavior_are_preserved() {
        let params =
            serde_json::json!({"prompt":"private_prompt_marker","key":"private_key_marker"});
        let input = format!(
            "{}\n{}\n{{\"private_malformed_marker\":\n",
            serde_json::json!({"id":"private_id_marker","method":"private_method_marker","params":params}),
            serde_json::json!({"method":"private_notification_marker","params":params})
        );
        let (logs, inbound, wire) = capture(&input);
        assert_eq!(inbound.len(), 2);
        match &inbound[0] {
            Inbound::Request {
                id,
                method,
                params: actual,
            } => {
                assert!(
                    matches!(id, acp::RequestId::Str(value) if value.as_str() == "private_id_marker")
                );
                assert_eq!(method, "private_method_marker");
                assert_eq!(actual.as_ref(), Some(&params));
            }
            _ => panic!("request was not delivered"),
        }
        match &inbound[1] {
            Inbound::Notification {
                method,
                params: actual,
            } => {
                assert_eq!(method, "private_notification_marker");
                assert_eq!(actual.as_ref(), Some(&params));
            }
            _ => panic!("notification was not delivered"),
        }
        assert!(logs.contains("ACP: parse error"));
        assert!(!logs.contains("private_"));
        let response: Value = serde_json::from_slice(&wire).unwrap();
        assert_eq!(
            response,
            serde_json::json!({"jsonrpc":"2.0","id":null,"error":acp::Error::parse_error()})
        );
        assert!(wire.ends_with(b"\n"));
    }
}
