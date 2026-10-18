//! Captured diagnostics from real dispatch and permission wire paths; no runtime startup.
use super::*;
use crate::connection::Inbound;
use crate::runtime::AgentRuntime;
use crate::server::AgentServer;
use claurst_core::{Config, Settings};
use serde_json::{json, Value};
use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::sync::Mutex as StdMutex;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, BufReader};

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

fn capture<F: Future>(future: F) -> (F::Output, String) {
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
    let output = tracing::subscriber::with_default(subscriber, || {
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), future)
                .await
                .unwrap()
        })
    });
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    (output, text)
}

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

fn check_unknown_dispatch_method(request: bool) {
    let (wire, logs) = capture(async {
        let (mut output, writer) = tokio::io::duplex(8192);
        let server = server(Connection::new(writer));
        let message = if request {
            Inbound::Request {
                id: acp::RequestId::Number(5),
                method: "private_method_marker-é".into(),
                params: None,
            }
        } else {
            Inbound::Notification {
                method: "private_method_marker-é".into(),
                params: None,
            }
        };
        server.dispatch(message).await.unwrap();
        drop(server);
        let mut wire = Vec::new();
        output.read_to_end(&mut wire).await.unwrap();
        wire
    });
    if request {
        assert_eq!(
            serde_json::from_slice::<Value>(&wire).unwrap(),
            json!({"jsonrpc":"2.0","id":5,"error":acp::Error::method_not_found()})
        );
    } else {
        assert!(wire.is_empty());
    }
    assert!(logs.contains(if request {
        "ACP: method not found"
    } else {
        "ACP: ignoring unknown notification"
    }));
    assert!(logs.contains(if request {
        "ACP: dispatch request"
    } else {
        "ACP: dispatch notification"
    }));
    assert!(!logs.contains("private_"), "method leaked: {logs}");
    assert!(logs.contains("method=\"unknown\""));
    assert!(logs.contains(&format!("method_bytes={}", "private_method_marker-é".len())));
}

#[test]
fn unknown_request_method_does_not_enter_logs() {
    check_unknown_dispatch_method(true);
}
#[test]
fn unknown_notification_method_does_not_enter_logs() {
    check_unknown_dispatch_method(false);
}

#[test]
fn malformed_cancel_logs_only_error_classification() {
    let value = json!("private_cancel_marker");
    let original = serde_json::from_value::<acp::CancelNotification>(value.clone()).unwrap_err();
    assert!(original.to_string().contains("private_cancel_marker"));
    let (_, logs) = capture(async {
        let server = server(Connection::new(tokio::io::sink()));
        server
            .dispatch(Inbound::Notification {
                method: "session/cancel".into(),
                params: Some(value),
            })
            .await
            .unwrap();
    });
    assert!(logs.contains("ACP: malformed session/cancel notification"));
    assert!(!logs.contains("private_"), "cancel data leaked: {logs}");
    assert!(logs.contains("category=Data"));
    assert!(logs.contains("line=0"));
    assert!(logs.contains("column=0"));
    assert!(logs.contains("method=\"session/cancel\""));
}

struct FailingWriter;
impl AsyncWrite for FailingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Ready(Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "private_io_marker",
        )))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[test]
fn failed_response_does_not_log_method_or_error_contents() {
    let (_, logs) = capture(async {
        server(Connection::new(FailingWriter))
            .dispatch(Inbound::Request {
                id: acp::RequestId::Number(5),
                method: "private_method_marker".into(),
                params: None,
            })
            .await
            .unwrap();
    });
    assert!(logs.contains("ACP: failed to send response"));
    assert!(
        !logs.contains("private_"),
        "response failure leaked: {logs}"
    );
    assert!(logs.contains("io_kind=Some(BrokenPipe)"));
}

async fn permission_reply(reply: Value) -> PermissionDecision {
    let (output, writer) = tokio::io::duplex(8192);
    let connection = Connection::new(writer);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let pending = PendingPermissionRequest {
        tool_use_id: "fixture-tool".into(),
        reason: "fixture approval".into(),
        decision_tx: Some(tx),
        request: PermissionRequest {
            tool_name: "Bash".into(),
            description: "fixture".into(),
            details: None,
            is_read_only: false,
            path: None,
            working_dir: None,
            allowed_roots: Vec::new(),
            context_description: None,
        },
    };
    let task = tokio::spawn(forward_pending(
        connection.clone(),
        acp::SessionId::new("fixture-session"),
        pending,
    ));
    let mut output = BufReader::new(output);
    let mut line = String::new();
    output.read_line(&mut line).await.unwrap();
    let request: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(request["method"], "session/request_permission");
    let mut reply = reply;
    reply["id"] = request["id"].clone();
    reply["jsonrpc"] = json!("2.0");
    let bytes = format!("{reply}\n");
    let (inbound, _rx) = tokio::sync::mpsc::unbounded_channel();
    crate::connection::run_reader(connection, bytes.as_bytes(), inbound)
        .await
        .unwrap();
    task.await.unwrap();
    rx.await.unwrap()
}

#[test]
fn permission_client_error_logs_code_without_message_or_data() {
    let (decision, logs) = capture(permission_reply(json!({"error":{
        "code":-32603,"message":"private_message_marker","data":{"private_key_marker":"private_data_marker"}
    }})));
    assert!(matches!(decision, PermissionDecision::Deny));
    assert!(logs.contains("ACP permission: client returned error, denying"));
    assert!(!logs.contains("private_"), "client error leaked: {logs}");
    assert!(logs.contains("code=-32603"));
    assert!(logs.contains("message_bytes=22"));
    assert!(logs.contains("has_data=true"));
}

#[test]
fn permission_decode_error_logs_only_category() {
    let value = json!({"outcome":{"outcome":"private_outcome_marker"}});
    let error =
        serde_json::from_value::<acp::RequestPermissionResponse>(value.clone()).unwrap_err();
    assert!(error.to_string().contains("private_outcome_marker"));
    let (decision, logs) = capture(permission_reply(json!({"result":value})));
    assert!(matches!(decision, PermissionDecision::Deny));
    assert!(logs.contains("ACP permission: send_request failed, denying"));
    assert!(!logs.contains("private_"), "typed response leaked: {logs}");
    assert!(logs.contains("category=Data"));
    assert!(logs.contains("line=0"));
    assert!(logs.contains("column=0"));
}

#[test]
fn valid_permission_and_cancel_controls_keep_behavior() {
    for outcome in [
        json!({"outcome":"selected","optionId":"allow_once"}),
        json!({"outcome":"cancelled"}),
    ] {
        let allowed = outcome["outcome"] == "selected";
        let (decision, logs) = capture(permission_reply(json!({"result":{"outcome":outcome}})));
        if allowed {
            assert!(matches!(decision, PermissionDecision::Allow));
        } else {
            assert!(matches!(decision, PermissionDecision::Deny));
        }
        assert!(!logs.contains("WARN"));
    }
    let (_, logs) = capture(async {
        let server = server(Connection::new(tokio::io::sink()));
        let session = crate::sessions::SessionState::new(
            acp::SessionId::new("fixture-session"),
            std::path::PathBuf::from("/unused-fixture"),
        );
        let token = session.cancel_token.lock().clone();
        server.sessions.insert(session);
        assert!(!token.is_cancelled());
        server
            .dispatch(Inbound::Notification {
                method: "session/cancel".into(),
                params: Some(json!({"sessionId":"fixture-session"})),
            })
            .await
            .unwrap();
        assert!(token.is_cancelled());
    });
    assert!(!logs.contains("WARN"));
}

#[test]
fn permission_io_error_is_denied_without_logging_error_contents() {
    let (decision, logs) = capture(async {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let pending = PendingPermissionRequest {
            tool_use_id: "fixture-tool".into(),
            reason: "fixture approval".into(),
            decision_tx: Some(tx),
            request: PermissionRequest {
                tool_name: "Bash".into(),
                description: "fixture".into(),
                details: None,
                is_read_only: false,
                path: None,
                working_dir: None,
                allowed_roots: Vec::new(),
                context_description: None,
            },
        };
        forward_pending(
            Connection::new(FailingWriter),
            acp::SessionId::new("fixture-session"),
            pending,
        )
        .await;
        rx.await.unwrap()
    });
    assert!(matches!(decision, PermissionDecision::Deny));
    assert!(logs.contains("ACP permission: send_request failed, denying"));
    assert!(logs.contains("error_class=\"io\""));
    assert!(logs.contains("io_kind=Some(BrokenPipe)"));
    assert!(
        !logs.contains("private_"),
        "permission transport error leaked: {logs}"
    );
}
