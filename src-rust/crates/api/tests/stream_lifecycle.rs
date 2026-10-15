use claurst_api::{
    client::ClientConfig, AnthropicClient, AnthropicStreamEvent, CreateMessageRequest,
    StreamHandler,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    time::timeout,
};

struct Handler {
    events: mpsc::UnboundedSender<AnthropicStreamEvent>,
    dropped: Option<oneshot::Sender<()>>,
}

impl StreamHandler for Handler {
    fn on_event(&self, event: &AnthropicStreamEvent) {
        let _ = self.events.send(event.clone());
    }
}

impl Drop for Handler {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

struct Fixture {
    receiver: mpsc::Receiver<AnthropicStreamEvent>,
    callbacks: mpsc::UnboundedReceiver<AnthropicStreamEvent>,
    dropped: oneshot::Receiver<()>,
    socket: TcpStream,
}

impl Fixture {
    async fn start(body: &str, complete: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = AnthropicClient::new(ClientConfig {
            api_key: "local-fixture".into(),
            api_base: format!("http://{}", listener.local_addr().unwrap()),
            request_timeout: Duration::from_secs(30),
            ..Default::default()
        })
        .unwrap();
        let (events, callbacks) = mpsc::unbounded_channel();
        let (dropped_tx, dropped) = oneshot::channel();
        let handler = Arc::new(Handler {
            events,
            dropped: Some(dropped_tx),
        });
        let serve = async {
            let (socket, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(socket);
            let mut request = String::new();
            reader.read_line(&mut request).await.unwrap();
            assert!(request.starts_with("POST /v1/messages "));
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
            }
            reader.read_exact(&mut vec![0; length]).await.unwrap();
            let length = if complete {
                body.len()
            } else {
                body.len() + 1_000_000
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}");
            reader
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
            reader.into_inner()
        };
        let request = CreateMessageRequest::builder("fixture", 16).build();
        let (receiver, socket) = timeout(Duration::from_secs(3), async {
            tokio::join!(client.create_message_stream(request, handler), serve)
        })
        .await
        .expect("local request did not connect");
        Self {
            receiver: receiver.unwrap(),
            callbacks,
            dropped,
            socket,
        }
    }

    async fn assert_stopped(mut self) {
        timeout(Duration::from_secs(1), &mut self.dropped)
            .await
            .expect("reader retained its handler after receiver cancellation")
            .unwrap();
        let mut byte = [0];
        let count = timeout(Duration::from_secs(1), self.socket.read(&mut byte))
            .await
            .expect("HTTP stream remained open")
            .unwrap();
        assert_eq!(count, 0, "expected client to close HTTP connection");
        assert!(
            self.callbacks.try_recv().is_err(),
            "unexpected callback after cancellation"
        );
    }
}

#[tokio::test]
async fn dropped_receiver_stops_idle_http_reader() {
    let mut fixture = Fixture::start("", false).await;
    // Replace and drop the actual receiver, retaining a closed dummy for the fixture.
    let (_, dummy) = mpsc::channel(1);
    drop(std::mem::replace(&mut fixture.receiver, dummy));
    fixture.assert_stopped().await;
}

#[tokio::test]
async fn closed_receiver_stops_idle_http_reader() {
    let mut fixture = Fixture::start("", false).await;
    fixture.receiver.close();
    fixture.assert_stopped().await;
}

#[tokio::test]
async fn closed_receiver_stops_backpressured_http_reader() {
    let mut fixture = Fixture::start(&"event: ping\ndata: {}\n\n".repeat(257), false).await;
    // The callback for event 257 precedes its blocked send into the 256-item channel.
    for _ in 0..257 {
        assert!(matches!(
            timeout(Duration::from_secs(1), fixture.callbacks.recv())
                .await
                .unwrap(),
            Some(AnthropicStreamEvent::Ping)
        ));
    }
    fixture.receiver.close();
    fixture.assert_stopped().await;
}

#[tokio::test]
async fn live_receiver_preserves_event_and_callback_delivery() {
    let mut fixture = Fixture::start(
        "event: ping\ndata: {}\n\nevent: message_stop\ndata: {}\n\n",
        true,
    )
    .await;
    let receiver = &mut fixture.receiver;
    assert!(matches!(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap(),
        Some(AnthropicStreamEvent::Ping)
    ));
    assert!(matches!(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap(),
        Some(AnthropicStreamEvent::MessageStop)
    ));
    assert!(timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        fixture.callbacks.recv().await,
        Some(AnthropicStreamEvent::Ping)
    ));
    assert!(matches!(
        fixture.callbacks.recv().await,
        Some(AnthropicStreamEvent::MessageStop)
    ));
    fixture.assert_stopped().await;
}

#[tokio::test]
async fn live_receiver_still_receives_http_body_errors() {
    let mut fixture = Fixture::start("", false).await;
    fixture.socket.shutdown().await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(1), fixture.receiver.recv()).await.unwrap(),
        Some(AnthropicStreamEvent::Error { error_type, .. }) if error_type == "stream_error"
    ));
    assert!(timeout(Duration::from_secs(1), fixture.receiver.recv())
        .await
        .unwrap()
        .is_none());
    fixture.assert_stopped().await;
}
