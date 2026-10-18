use claurst_api::{
    client::ClientConfig, AnthropicClient, AnthropicStreamEvent, CreateMessageRequest,
    StreamHandler,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    time::timeout,
};

struct Handler;
impl StreamHandler for Handler {
    fn on_event(&self, _: &AnthropicStreamEvent) {}
}

async fn events(body: &str) -> Vec<AnthropicStreamEvent> {
    timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = AnthropicClient::new(ClientConfig {
            api_key: "owned-loopback-fixture".into(),
            api_base: format!("http://{}", listener.local_addr().unwrap()),
            max_retries: 0,
            request_timeout: Duration::from_secs(3),
            ..Default::default()
        }).unwrap();
        let serve = async {
            let (socket, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(socket);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert!(line.starts_with("POST /v1/messages "));
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
            reader.read_exact(&mut vec![0; length.unwrap()]).await.unwrap();
            // Exact length makes this a complete HTTP body, not transport truncation.
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            reader.get_mut().write_all(response.as_bytes()).await.unwrap();
            reader.get_mut().shutdown().await.unwrap();
        };
        let collect = async {
            let request = CreateMessageRequest::builder("fixture", 16).build();
            let mut receiver = client.create_message_stream(request, Arc::new(Handler)).await.unwrap();
            let mut events = Vec::new();
            while let Some(event) = receiver.recv().await { events.push(event); }
            events
        };
        let ((), events) = tokio::join!(serve, collect);
        events
    }).await.expect("bounded loopback stream did not finish")
}

#[tokio::test]
async fn empty_complete_http_body_is_not_a_complete_message() {
    let events = events("").await;
    assert!(
        matches!(events.as_slice(), [AnthropicStreamEvent::Error { error_type, .. }] if error_type == "stream_error"),
        "{events:?}"
    );
}

#[tokio::test]
async fn partial_sse_requires_message_stop() {
    let events = events("event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial answer\"}}\n\n").await;
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(matches!(
        &events[0],
        AnthropicStreamEvent::ContentBlockDelta { .. }
    ));
    assert!(
        matches!(&events[1], AnthropicStreamEvent::Error { error_type, .. } if error_type == "stream_error")
    );
}

#[tokio::test]
async fn message_stop_completes_without_synthetic_error() {
    let events = events("event: ping\ndata: {}\n\nevent: message_stop\ndata: {}\n\n").await;
    assert!(matches!(
        events.as_slice(),
        [
            AnthropicStreamEvent::Ping,
            AnthropicStreamEvent::MessageStop
        ]
    ));
}

#[tokio::test]
async fn explicit_provider_error_is_not_duplicated_at_eof() {
    let events = events("event: error\ndata: {\"error\":{\"type\":\"overloaded_error\",\"message\":\"fixture overload\"}}\n\n").await;
    assert!(
        matches!(events.as_slice(), [AnthropicStreamEvent::Error { error_type, message }] if error_type == "overloaded_error" && message == "fixture overload")
    );
}
