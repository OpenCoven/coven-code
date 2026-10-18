use claurst_api::{
    client::ClientConfig, AnthropicClient, AnthropicStreamEvent, CreateMessageRequest,
    StreamHandler,
};
use claurst_core::error::ClaudeError;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::Notify,
    time::timeout,
};

struct Handler;
impl StreamHandler for Handler {
    fn on_event(&self, _: &AnthropicStreamEvent) {}
}
#[derive(Clone, Copy)]
enum Body {
    Complete,
    Truncated,
    Stalled,
}
struct Reply {
    status: u16,
    body: Body,
    retry_after: Option<u64>,
}

async fn request(
    replies: &[Reply],
    retries: u32,
) -> Result<Vec<AnthropicStreamEvent>, ClaudeError> {
    timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = AnthropicClient::new(ClientConfig {
            api_key: "owned-loopback-fixture".into(),
            api_base: format!("http://{}/private-url-marker?secret=private-url-marker", listener.local_addr().unwrap()),
            max_retries: retries, request_timeout: Duration::from_millis(500),
            ..Default::default()
        }).unwrap();
        let finished = Notify::new();
        let serve = async {
            for reply in replies {
                let (socket, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(socket);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                assert!(line.starts_with("POST /private-url-marker?secret=private-url-marker/v1/messages "));
                let mut length = None;
                loop {
                    line.clear();
                    assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
                    if line == "\r\n" { break; }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = Some(value.trim().parse::<usize>().unwrap());
                        }
                    }
                }
                let mut body = vec![0; length.unwrap()];
                reader.read_exact(&mut body).await.unwrap();
                let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(parsed["model"], "fixture");
                assert_eq!(parsed["stream"], true);
                let text = if reply.status == 200 {
                    "event: message_stop\ndata: {}\n\n"
                } else {
                    "{\"type\":\"error\",\"error\":{\"type\":\"fixture_error\",\"message\":\"fixture provider message\"}}"
                };
                let (sent, length) = match reply.body {
                    Body::Complete => (text, text.len()),
                    Body::Truncated | Body::Stalled => ("private-partial-body", 4096),
                };
                let retry = reply.retry_after.map(|value| format!("Retry-After: {value}\r\n")).unwrap_or_default();
                let wire = format!("HTTP/1.1 {} Fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\n{retry}Connection: close\r\n\r\n{sent}", reply.status);
                reader.get_mut().write_all(wire.as_bytes()).await.unwrap();
                if matches!(reply.body, Body::Stalled) { finished.notified().await; }
                reader.get_mut().shutdown().await.unwrap();
            }
            // The script is the exact expected request count; extra retries fail.
            assert!(timeout(Duration::from_millis(100), listener.accept()).await.is_err(), "unexpected additional request");
        };
        let call = async {
            let result = async {
                let mut receiver = client.create_message_stream(CreateMessageRequest::builder("fixture", 16).build(), Arc::new(Handler)).await?;
                let mut events = Vec::new();
                while let Some(event) = receiver.recv().await { events.push(event); }
                Ok(events)
            }.await;
            finished.notify_one();
            result
        };
        let ((), result) = tokio::join!(serve, call);
        result
    }).await.expect("bounded loopback error-response fixture stalled")
}

async fn failed_body(status: u16, body: Body) {
    let error = request(
        &[Reply {
            status,
            body,
            retry_after: Some(37),
        }],
        0,
    )
    .await
    .unwrap_err();
    let message = match error {
        ClaudeError::RateLimit {
            retry_after_secs,
            message,
        } if status == 429 => {
            assert_eq!(retry_after_secs, Some(37));
            message.expect("body failure must have a diagnostic")
        }
        ClaudeError::ApiStatus {
            status: actual,
            message,
        } => {
            assert_eq!(actual, status);
            message
        }
        other => panic!("HTTP classification changed: {other:?}"),
    };
    assert!(message.contains(&format!("HTTP {status}")), "{message}");
    assert!(message.contains("error response body"), "{message}");
    assert!(!message.contains("private-partial-body"), "{message}");
    assert!(!message.contains("private-url-marker"), "{message}");
    if matches!(body, Body::Stalled) {
        assert!(message.contains("timed out"), "{message}");
    }
}

macro_rules! failed_body_test {
    ($name:ident, $status:expr, $body:expr) => {
        #[tokio::test]
        async fn $name() {
            failed_body($status, $body).await;
        }
    };
}
failed_body_test!(truncated_400, 400, Body::Truncated);
failed_body_test!(truncated_429, 429, Body::Truncated);
failed_body_test!(truncated_529, 529, Body::Truncated);
failed_body_test!(stalled_400, 400, Body::Stalled);
failed_body_test!(stalled_429, 429, Body::Stalled);
failed_body_test!(stalled_529, 529, Body::Stalled);

#[tokio::test]
async fn complete_errors_preserve_classification_and_messages() {
    for status in [400, 401, 429, 529] {
        let error = request(
            &[Reply {
                status,
                body: Body::Complete,
                retry_after: Some(37),
            }],
            0,
        )
        .await
        .unwrap_err();
        match (status, error) {
            (401, ClaudeError::Auth(message)) => assert_eq!(message, "fixture provider message"),
            (
                429,
                ClaudeError::RateLimit {
                    retry_after_secs,
                    message,
                },
            ) => {
                assert_eq!(retry_after_secs, Some(37));
                assert_eq!(message.as_deref(), Some("fixture provider message"));
            }
            (
                400 | 529,
                ClaudeError::ApiStatus {
                    status: actual,
                    message,
                },
            ) => {
                assert_eq!(actual, status);
                assert_eq!(
                    message,
                    if status == 529 {
                        "Overloaded: fixture provider message"
                    } else {
                        "fixture provider message"
                    }
                );
            }
            (_, other) => panic!("unexpected classification: {other:?}"),
        }
    }
}

#[tokio::test]
async fn short_retry_still_reaches_success_without_reading_error_body() {
    for status in [429, 529] {
        let events = request(
            &[
                Reply {
                    status,
                    body: Body::Truncated,
                    retry_after: Some(0),
                },
                Reply {
                    status: 200,
                    body: Body::Complete,
                    retry_after: None,
                },
            ],
            1,
        )
        .await
        .unwrap();
        assert!(matches!(
            events.as_slice(),
            [AnthropicStreamEvent::MessageStop]
        ));
    }
}

#[tokio::test]
async fn long_retry_after_surfaces_without_silent_retry() {
    let error = request(
        &[Reply {
            status: 429,
            body: Body::Complete,
            retry_after: Some(37),
        }],
        1,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        ClaudeError::RateLimit {
            retry_after_secs: Some(37),
            ..
        }
    ));
}

#[tokio::test]
async fn exhausted_retry_keeps_zero_retry_after_and_body_diagnostic() {
    let error = request(
        &[
            Reply {
                status: 429,
                body: Body::Complete,
                retry_after: Some(0),
            },
            Reply {
                status: 429,
                body: Body::Truncated,
                retry_after: Some(0),
            },
        ],
        1,
    )
    .await
    .unwrap_err();
    match error {
        ClaudeError::RateLimit {
            retry_after_secs,
            message,
        } => {
            assert_eq!(retry_after_secs, Some(0));
            let message = message.expect("exhausted retry must retain body failure diagnostic");
            assert!(message.contains("HTTP 429"), "{message}");
            assert!(message.contains("error response body"), "{message}");
        }
        other => panic!("retry classification changed: {other:?}"),
    }
}
