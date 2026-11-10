//! Real streamable HTTP initialization failures, with and without owned shutdown.
use super::*;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

const SECRET: &str = "streamable-http-private-canary";
const TOKEN: &str = "streamable-http-bearer-canary";

async fn initialization_error(url: String, owned: bool) -> anyhow::Error {
    let config = claurst_core::config::McpServerConfig {
        name: "http-privacy-fixture".into(),
        command: None,
        args: vec![],
        env: HashMap::new(),
        url: Some(url),
        server_type: "http".into(),
    };
    let (_stop, shutdown) = watch::channel(false);
    match timeout(
        Duration::from_secs(5),
        RmcpClientBackend::connect_http(
            &config,
            Some(TOKEN.into()),
            rmcp_model::ProtocolVersion::default(),
            owned.then_some(shutdown),
        ),
    )
    .await
    .expect("HTTP initialization and retirement must terminate")
    {
        Ok(_) => panic!("fixture initialization must fail"),
        Err(error) => error,
    }
}

fn assert_private(error: &anyhow::Error, category: &str) {
    for rendered in [
        error.to_string(),
        format!("{error:#}"),
        format!("{error:?}"),
    ] {
        for secret in [SECRET, TOKEN, "http://", "127.0.0.1"] {
            assert!(!rendered.contains(secret), "leaked {secret}: {rendered}");
        }
        assert!(rendered.contains("http-privacy-fixture"), "{rendered}");
        assert!(
            rendered.contains(category),
            "safe category missing: {rendered}"
        );
    }
}

#[tokio::test]
async fn http_refused_initialization_does_not_expose_endpoint() {
    for owned in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let error = initialization_error(
            format!("http://{address}/messages?key={SECRET}#{SECRET}"),
            owned,
        )
        .await;
        assert_private(&error, "connection");
    }
}

#[tokio::test]
async fn http_forbidden_initialization_does_not_expose_response_body() {
    for owned in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(stream.read_u8().await.unwrap());
                assert!(bytes.len() < 16384);
            }
            let headers = String::from_utf8(bytes).unwrap();
            assert!(headers.starts_with(&format!("POST /messages?key={SECRET} HTTP/1.1")));
            assert!(headers
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {TOKEN}")));
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            assert!(length < 16384);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            let request: crate::types::JsonRpcRequest = serde_json::from_slice(&body).unwrap();
            assert_eq!(request.method, "initialize");
            let response = format!("HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{SECRET}", SECRET.len());
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        let error =
            initialization_error(format!("http://{address}/messages?key={SECRET}"), owned).await;
        server.await.unwrap();
        assert_private(&error, "403");
    }
}

async fn post_fixture(
    status: &'static str,
    content_type: &'static str,
    body: String,
    extra_headers: &'static str,
) -> Result<StreamableHttpPostResponse, StreamableHttpError<reqwest::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(stream.read_u8().await.unwrap());
            assert!(bytes.len() < 16384);
        }
        let headers = String::from_utf8(bytes).unwrap();
        assert!(headers.starts_with(&format!("POST /messages?key={SECRET} HTTP/1.1")));
        let lower = headers.to_ascii_lowercase();
        assert!(lower.contains(&format!("authorization: bearer {TOKEN}")));
        assert!(lower.contains("mcp-session-id: fixture-session"));
        assert!(lower.contains("x-fixture: preserved"));
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        assert!(length < 16384);
        let mut request = vec![0; length];
        stream.read_exact(&mut request).await.unwrap();
        let message: crate::types::JsonRpcRequest = serde_json::from_slice(&request).unwrap();
        assert_eq!(message.method, "tools/list");
        assert_eq!(message.id, Some(serde_json::json!(17)));
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}", body.len());
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    let message = serde_json::from_str::<rmcp_model::ClientJsonRpcMessage>(
        r#"{"jsonrpc":"2.0","id":17,"method":"tools/list"}"#,
    )
    .unwrap();
    let result = timeout(
        Duration::from_secs(5),
        reqwest::Client::new().post_message(
            format!("http://{address}/messages?key={SECRET}").into(),
            message,
            Some("fixture-session".into()),
            Some(TOKEN.into()),
            HashMap::from([("x-fixture".parse().unwrap(), "preserved".parse().unwrap())]),
        ),
    )
    .await
    .unwrap();
    server.await.unwrap();
    result
}

#[tokio::test]
async fn http_protocol_responses_keep_request_auth_session_and_json_bytes() {
    let response = post_fixture(
        "200 OK",
        "application/json",
        r#"{"jsonrpc":"2.0","id":17,"result":{"tools":[]}}"#.into(),
        "Mcp-Session-Id: returned-session\r\n",
    )
    .await
    .unwrap();
    let StreamableHttpPostResponse::Json(message, session) = response else {
        panic!("successful JSON response must remain JSON");
    };
    assert_eq!(session.as_deref(), Some("returned-session"));
    assert!(matches!(message, rmcp_model::JsonRpcMessage::Response(_)));

    // JSON-RPC errors are protocol results, not opaque HTTP diagnostic bodies.
    let response = post_fixture(
        "400 Bad Request",
        "application/json",
        r#"{"jsonrpc":"2.0","id":17,"error":{"code":-32600,"message":"request rejected"}}"#.into(),
        "",
    )
    .await
    .unwrap();
    let StreamableHttpPostResponse::Json(message, _) = response else {
        panic!("parsed JSON-RPC error must remain protocol data");
    };
    let rmcp_model::JsonRpcMessage::Error(error) = message else {
        panic!("error response must keep its type");
    };
    assert_eq!(error.error.message, "request rejected");
}

#[tokio::test]
async fn http_auth_and_session_error_types_remain_available() {
    let required = post_fixture(
        "401 Unauthorized",
        "text/plain",
        SECRET.into(),
        "WWW-Authenticate: Bearer realm=\"fixture\"\r\n",
    )
    .await
    .unwrap_err();
    let StreamableHttpError::AuthRequired(required) = required else {
        panic!("auth challenge must remain typed");
    };
    assert_eq!(required.www_authenticate_header, "Bearer realm=\"fixture\"");
    let scope = post_fixture(
        "403 Forbidden",
        "text/plain",
        SECRET.into(),
        "WWW-Authenticate: Bearer error=\"insufficient_scope\", scope=\"fixture.read\"\r\n",
    )
    .await
    .unwrap_err();
    let StreamableHttpError::InsufficientScope(scope) = scope else {
        panic!("scope challenge must remain typed");
    };
    assert_eq!(scope.required_scope.as_deref(), Some("fixture.read"));
    assert!(matches!(
        post_fixture("404 Not Found", "text/plain", SECRET.into(), "").await,
        Err(StreamableHttpError::SessionExpired)
    ));
}

#[tokio::test]
async fn http_notifications_and_malformed_json_keep_accepted_semantics() {
    for status in ["202 Accepted", "204 No Content", "200 OK"] {
        let response = post_fixture(status, "application/json", SECRET.into(), "")
            .await
            .unwrap();
        assert!(matches!(response, StreamableHttpPostResponse::Accepted));
    }
}

#[tokio::test]
async fn http_transport_errors_drop_body_and_sources_in_every_format() {
    for content_type in ["text/plain", "application/json"] {
        let error = post_fixture("500 Internal Server Error", content_type, SECRET.into(), "")
            .await
            .unwrap_err();
        for rendered in [error.to_string(), format!("{error:?}")] {
            assert!(rendered.contains("500"), "{rendered}");
            assert!(!rendered.contains(SECRET), "{rendered}");
            assert!(!rendered.contains(TOKEN), "{rendered}");
        }
        assert!(std::error::Error::source(&error).is_none());
    }
}
