//! Actual legacy GET/discovered-POST failures must not disclose credentials.
use super::*;
use rmcp::transport::Transport;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};

const SECRET: &str = "mcp-endpoint-privacy-canary";
const TOKEN: &str = "mcp-bearer-canary";

fn config(url: String) -> claurst_core::config::McpServerConfig {
    claurst_core::config::McpServerConfig {
        name: "privacy-fixture".into(),
        command: None,
        args: vec![],
        env: HashMap::new(),
        url: Some(url),
        server_type: "sse".into(),
    }
}

fn assert_private(error: &anyhow::Error, stage: &str, category: &str) {
    for rendered in [
        error.to_string(),
        format!("{error:#}"),
        format!("{error:?}"),
    ] {
        assert!(rendered.contains("privacy-fixture"), "missing fixture name");
        assert!(rendered.contains(stage), "missing transport stage");
        assert!(rendered.contains(category), "missing failure category");
        for secret in [SECRET, TOKEN, "127.0.0.1", "http://", "userinfo-secret"] {
            assert!(
                !rendered.contains(secret),
                "rendered error contains private fixture data"
            );
        }
    }
    assert_eq!(error.chain().count(), 1, "raw sources must not escape");
}

async fn connect_error(url: String) -> anyhow::Error {
    match timeout(
        Duration::from_secs(5),
        LegacySseRmcpTransport::connect(&config(url), Some(TOKEN.into()), None),
    )
    .await
    .expect("GET must terminate")
    {
        Ok(_) => panic!("fixture GET must fail"),
        Err(error) => error,
    }
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(5), async {
        loop {
            let byte = stream.read_u8().await.unwrap();
            bytes.push(byte);
            assert!(bytes.len() < 16384, "bounded fixture headers");
            if bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let header = String::from_utf8(bytes.clone()).unwrap();
        let length = header
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        assert!(length < 16384, "bounded fixture body");
        let mut body = vec![0; length];
        stream.read_exact(&mut body).await.unwrap();
        bytes.extend_from_slice(&body);
    })
    .await
    .unwrap();
    String::from_utf8(bytes).unwrap()
}

async fn respond(stream: &mut TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
}

fn message() -> rmcp::service::TxJsonRpcMessage<RoleClient> {
    serde_json::from_value(json!({"jsonrpc":"2.0", "id": 17, "method":"tools/list"})).unwrap()
}

#[tokio::test]
async fn get_refusal_redacts_userinfo_query_and_fragment() {
    // Release a local port immediately before the refused connection.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = socket.local_addr().unwrap();
    drop(socket);
    let error = connect_error(format!(
        "http://{SECRET}:userinfo-secret@{addr}/sse?key={SECRET}#{SECRET}"
    ))
    .await;
    assert_private(&error, "GET", "connection");
}

#[tokio::test]
async fn get_builder_failure_redacts_url() {
    let error = connect_error(format!(
        "http://{SECRET}:userinfo-secret@[bad-address]/sse?key={SECRET}"
    ))
    .await;
    assert_private(&error, "GET", "builder");
}

#[tokio::test]
async fn discovered_post_refusal_redacts_userinfo_query_and_fragment() {
    let refused = tokio::net::TcpSocket::new_v4().unwrap();
    refused.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let endpoint = format!(
        "http://{SECRET}:userinfo-secret@{}/messages?key={SECRET}#{SECRET}",
        refused.local_addr().unwrap()
    );
    drop(refused);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sse?get={SECRET}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let request = read_request(&mut stream).await;
        assert!(request.starts_with(&format!("GET /sse?get={SECRET} HTTP/1.1\r\n")));
        assert!(request
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {TOKEN}")));
        respond(
            &mut stream,
            "text/event-stream",
            &format!("event: endpoint\ndata: {endpoint}\n\n"),
        )
        .await;
    });
    let mut transport = timeout(
        Duration::from_secs(5),
        LegacySseRmcpTransport::connect(&config(url), Some(TOKEN.into()), None),
    )
    .await
    .unwrap()
    .unwrap();
    let error = timeout(Duration::from_secs(5), transport.send(message()))
        .await
        .unwrap()
        .unwrap_err();
    assert_private(&anyhow::Error::from(error), "POST", "connection");
    transport.close().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn successful_discovery_preserves_get_post_auth_and_json() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/sse?key={SECRET}");
    let server = tokio::spawn(async move {
        let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let get = read_request(&mut stream).await;
        assert!(get.starts_with(&format!("GET /sse?key={SECRET} HTTP/1.1\r\n")));
        assert!(get
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {TOKEN}")));
        respond(
            &mut stream,
            "text/event-stream",
            &format!("event: endpoint\ndata: /messages?post={SECRET}\n\n"),
        )
        .await;
        let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let post = read_request(&mut stream).await;
        assert!(post.starts_with(&format!("POST /messages?post={SECRET} HTTP/1.1\r\n")));
        assert!(post
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {TOKEN}")));
        let (_, body) = post.split_once("\r\n\r\n").unwrap();
        let request: crate::types::JsonRpcRequest = serde_json::from_str(body).unwrap();
        assert_eq!(request.method, "tools/list");
        respond(
            &mut stream,
            "application/json",
            r#"{"jsonrpc":"2.0","id":17,"result":{"tools":[]}}"#,
        )
        .await;
    });
    let mut transport = timeout(
        Duration::from_secs(5),
        LegacySseRmcpTransport::connect(&config(url), Some(TOKEN.into()), None),
    )
    .await
    .unwrap()
    .unwrap();
    timeout(Duration::from_secs(5), transport.send(message()))
        .await
        .unwrap()
        .unwrap();
    let response = timeout(Duration::from_secs(5), transport.receive())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        response,
        rmcp::service::RxJsonRpcMessage::<RoleClient>::Response(_)
    ));
    transport.close().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn malformed_discovered_endpoint_does_not_echo_announcement() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sse?key={SECRET}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        read_request(&mut stream).await;
        respond(&mut stream, "text/event-stream", &format!("event: endpoint\ndata: http://{SECRET}:userinfo-secret@[invalid]/post?key={SECRET}\n\n")).await;
    });
    let error = connect_error(url).await;
    assert_private(&error, "endpoint", "invalid");
    server.await.unwrap();
}

#[tokio::test]
async fn get_redirect_exhaustion_does_not_echo_destination() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sse?key={SECRET}", listener.local_addr().unwrap());
    let location = url.clone();
    let server = tokio::spawn(async move {
        for _ in 0..11 {
            let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let request = read_request(&mut stream).await;
            assert!(request.starts_with(&format!("GET /sse?key={SECRET} HTTP/1.1\r\n")));
            stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        }
    });
    let error = connect_error(url).await;
    assert_private(&error, "GET", "redirect");
    server.await.unwrap();
}

#[tokio::test]
async fn post_truncated_body_does_not_echo_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sse?key={SECRET}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        read_request(&mut stream).await;
        respond(
            &mut stream,
            "text/event-stream",
            &format!("event: endpoint\ndata: /messages?key={SECRET}\n\n"),
        )
        .await;
        let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        read_request(&mut stream).await;
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();
        stream.shutdown().await.unwrap();
    });
    let mut transport = timeout(
        Duration::from_secs(5),
        LegacySseRmcpTransport::connect(&config(url), Some(TOKEN.into()), None),
    )
    .await
    .unwrap()
    .unwrap();
    let error = timeout(Duration::from_secs(5), transport.send(message()))
        .await
        .unwrap()
        .unwrap_err();
    assert_private(&anyhow::Error::from(error), "POST response body", "decode");
    transport.close().await.unwrap();
    server.await.unwrap();
}
