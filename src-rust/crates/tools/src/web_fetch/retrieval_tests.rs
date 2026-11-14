use super::*;
use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use std::path::PathBuf;
use std::sync::{atomic::AtomicUsize, Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut bytes = [0; 1024];
    while !request.ends_with(b"\r\n\r\n") {
        let size = stream.read(&mut bytes).await.unwrap();
        assert!(size > 0, "fixture received incomplete HTTP headers");
        request.extend_from_slice(&bytes[..size]);
        assert!(
            request.len() <= 8192,
            "fixture HTTP headers exceeded their bound"
        );
    }
    String::from_utf8(request).unwrap()
}

async fn fixture(responses: Vec<Vec<u8>>) -> (Server, reqwest::Url, reqwest::Client) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server(tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _request = read_request(&mut stream).await;
            let _ = stream.write_all(&response).await;
        }
    }));
    let url =
        reqwest::Url::parse(&format!("http://fetch.example:{}/page", address.port())).unwrap();
    let client = network::client_builder()
        .resolve("fetch.example", address)
        .build()
        .unwrap();
    (server, url, client)
}

fn context() -> ToolContext {
    let mode = PermissionMode::BypassPermissions;
    ToolContext {
        working_dir: PathBuf::from("."),
        permission_mode: mode.clone(),
        permission_handler: Arc::new(AutoPermissionHandler { mode }),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: "webfetch-fixture".into(),
        goal_turn: Default::default(),
        file_history: Arc::new(parking_lot::Mutex::new(
            claurst_core::file_history::FileHistory::new(),
        )),
        current_turn: Arc::new(AtomicUsize::new(0)),
        non_interactive: true,
        mcp_manager: None,
        config: Config::default(),
        managed_agent_config: None,
        completion_notifier: None,
        pending_permissions: None,
        permission_manager: None,
        user_question_tx: None,
    }
}
fn response(body: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).into_bytes()
}

#[tokio::test]
async fn fresh_body_cannot_be_replaced_by_an_old_extraction() {
    let (_server, url, client) = fixture(vec![response("fresh body")]).await;
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(format!("{}.txt", url_hash(url.as_str()))),
        "stale body",
    )
    .unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, temp.path()).await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "fresh body");
}

#[tokio::test]
async fn a_utf8_character_crossing_the_output_boundary_does_not_panic() {
    let text = format!("{}界tail", "a".repeat(99_999));
    let (_server, url, client) = fixture(vec![response(&text)]).await;
    let temp = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, temp.path()).await;
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.starts_with(&"a".repeat(99_999)));
}

#[tokio::test]
async fn an_unbounded_chunked_body_is_rejected_before_output_truncation() {
    let body = "x".repeat(2 * 1024 * 1024 + 1);
    let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",body.len()).into_bytes();
    let (_server, url, client) = fixture(vec![response]).await;
    let temp = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, temp.path()).await;
    assert!(
        result.is_error,
        "the network body must be bounded before truncating displayed text"
    );
    assert!(result.content.contains("limit"), "{}", result.content);
}

#[tokio::test]
async fn each_cache_use_requires_a_fresh_response_and_unchanged_body() {
    let (mut server, url, client) = fixture(vec![
        response("first"),
        response("second"),
        response("second"),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    for (text, cached) in [("first", false), ("second", false), ("second", true)] {
        let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content, text);
        let metadata: Retrieval = serde_json::from_value(result.metadata.unwrap()).unwrap();
        assert_eq!(metadata.schema_version, 1);
        assert_eq!(metadata.requested_url, url.as_str());
        assert_eq!(metadata.final_url, url.as_str());
        assert_eq!(metadata.http_status, 200);
        assert_eq!(metadata.content_type.as_deref(), Some("text/plain"));
        assert!(chrono::DateTime::parse_from_rfc3339(&metadata.retrieved_at).is_ok());
        assert_eq!(
            metadata.body_sha256,
            format!("{:x}", Sha256::digest(text.as_bytes()))
        );
        assert_eq!(metadata.body_bytes, text.len());
        assert_eq!(metadata.extraction, Extraction::PlainText);
        assert_eq!(metadata.extraction_cached, cached);
        assert!(!metadata.text_truncated);
        assert_eq!(metadata.original_text_bytes, text.len());
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), &mut server.0)
        .await
        .expect("all three responses must actually be requested")
        .unwrap();
}

#[tokio::test]
async fn content_type_changes_invalidate_an_identical_body_extraction() {
    let body = format!(
        "<body>İ<script>PRIVATE_SCRIPT_CANARY</script>Visible {}</body>",
        "word ".repeat(110)
    );
    let html=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).into_bytes();
    let (_server, url, client) = fixture(vec![html, response(&body)]).await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let first = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert!(!first.is_error, "{}", first.content);
    assert!(!first.content.contains("PRIVATE_SCRIPT_CANARY"));
    assert!(first.content.contains("Visible"));
    let first: Retrieval = serde_json::from_value(first.metadata.unwrap()).unwrap();
    assert_eq!(first.extraction, Extraction::HtmlToText);
    let second = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert_eq!(second.content, body);
    let second: Retrieval = serde_json::from_value(second.metadata.unwrap()).unwrap();
    assert_eq!(second.extraction, Extraction::PlainText);
    assert!(!second.extraction_cached);
}

#[tokio::test]
async fn declared_oversize_is_rejected_without_reading_a_body() {
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_BODY_BYTES + 1
    )
    .into_bytes();
    let (_server, url, client) = fixture(vec![headers]).await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert!(result.is_error);
    assert!(result.content.contains("limit"), "{}", result.content);
}

#[tokio::test]
async fn the_exact_body_limit_is_accepted_and_output_truncation_is_truthful() {
    let body = "x".repeat(MAX_BODY_BYTES);
    let (_server, url, client) = fixture(vec![response(&body)]).await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert!(!result.is_error, "{}", result.content);
    let metadata: Retrieval = serde_json::from_value(result.metadata.unwrap()).unwrap();
    assert_eq!(metadata.body_bytes, MAX_BODY_BYTES);
    assert_eq!(metadata.original_text_bytes, MAX_BODY_BYTES);
    assert!(metadata.text_truncated);
    assert!(result.content.starts_with(&"x".repeat(MAX_OUTPUT_BYTES)));
    assert!(result.content.contains("total UTF-8 bytes"));
}

#[tokio::test]
async fn redirect_to_private_destination_is_stopped_before_contact() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let private = listener.local_addr().unwrap();
    let redirect=format!("HTTP/1.1 302 Found\r\nLocation: http://{private}/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes();
    let (_server, url, client) = fixture(vec![redirect]).await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert!(result.is_error);
    assert!(result.content.contains("non-public"), "{}", result.content);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept())
            .await
            .is_err(),
        "the forbidden destination must receive no connection"
    );
}

#[tokio::test]
async fn a_public_redirect_reports_the_actual_final_url() {
    let (_final_server, final_url, _) = fixture(vec![response("final body")]).await;
    let redirect=format!("HTTP/1.1 302 Found\r\nLocation: {final_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes();
    let (_server, url, client) = fixture(vec![redirect]).await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "final body");
    let metadata: Retrieval = serde_json::from_value(result.metadata.unwrap()).unwrap();
    assert_eq!(metadata.requested_url, url.as_str());
    assert_eq!(metadata.final_url, final_url.as_str());
    assert_ne!(metadata.requested_url, metadata.final_url);
}

#[tokio::test]
async fn a_redirect_cycle_hits_the_actual_client_limit() {
    let redirect =
        b"HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec();
    let (_server, url, client) = fixture(vec![redirect; 12]).await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("redirect limit"),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn basic_auth_and_query_targets_are_preserved_but_provenance_drops_credentials() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _server = Server(tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await;
        stream.write_all(&response("auth body")).await.unwrap();
        tx.send(request).unwrap();
    }));
    let url = reqwest::Url::parse(&format!(
        "http://user:password@fetch.example:{}/page?token=canary#fragment",
        address.port()
    ))
    .unwrap();
    let client = network::client_builder()
        .resolve("fetch.example", address)
        .build()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert!(!result.is_error, "{}", result.content);
    let actual = rx.await.unwrap();
    assert!(
        actual.starts_with("GET /page?token=canary HTTP/1.1\r\n"),
        "{actual}"
    );
    assert!(actual
        .to_ascii_lowercase()
        .contains("authorization: basic dxnlcjpwyxnzd29yza=="));
    let metadata: Retrieval = serde_json::from_value(result.metadata.unwrap()).unwrap();
    assert!(metadata.url_credentials_redacted);
    assert!(metadata.url_fragments_omitted);
    for url in [&metadata.requested_url, &metadata.final_url] {
        assert!(!url.contains("password") && !url.contains("user@") && !url.contains("fragment"));
    }
}

#[test]
fn semantic_excerpt_and_output_cuts_are_utf8_boundaries() {
    for limit in [20_000, MAX_OUTPUT_BYTES] {
        let text = format!("{}界after", "x".repeat(limit - 1));
        assert_eq!(utf8_prefix(&text, limit).len(), limit - 1);
        assert!(text.is_char_boundary(utf8_prefix(&text, limit).len()));
    }
}

#[tokio::test]
async fn cached_model_extraction_is_still_labeled_as_model_rewritten() {
    let body = "<body>source page</body>";
    let reply=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).into_bytes();
    let (_server, url, client) = fixture(vec![reply]).await;
    let directory = tempfile::tempdir().unwrap();
    let extracted = cache::Extracted {
        text: "model summary".into(),
        method: Extraction::ClaudeHaiku,
        original_text_bytes: 13,
        text_truncated: false,
    };
    cache::save(
        directory.path(),
        url.as_str(),
        &format!("{:x}", Sha256::digest(body.as_bytes())),
        "text/html",
        &extracted,
    )
    .unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert!(!result.is_error, "{}", result.content);
    assert!(result
        .content
        .starts_with("[Model-extracted content: Claude Haiku; not verbatim page text]"));
    assert!(result.content.ends_with("model summary"));
    let metadata: Retrieval = serde_json::from_value(result.metadata.unwrap()).unwrap();
    assert_eq!(metadata.extraction, Extraction::ClaudeHaiku);
    assert!(metadata.extraction_cached);
}

#[tokio::test]
async fn a_corrupt_or_oversize_cache_cannot_override_fresh_content() {
    let (_server, url, client) = fixture(vec![response("first"), response("second")]).await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join(format!("{}.json", url_hash(url.as_str())));
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    for invalid in [false, true] {
        if invalid {
            std::fs::write(&path, "{not json}").unwrap();
        } else {
            std::fs::File::create(&path)
                .unwrap()
                .set_len(4 * 1024 * 1024)
                .unwrap();
        }
        let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content, if invalid { "second" } else { "first" });
        let metadata: Retrieval = serde_json::from_value(result.metadata.unwrap()).unwrap();
        assert!(!metadata.extraction_cached);
    }
}

#[tokio::test]
async fn invalid_utf8_is_decoded_with_an_explicit_replacement_receipt() {
    let mut reply=b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\n".to_vec();
    reply.extend_from_slice(&[b'a', 0xff]);
    let (_server, url, client) = fixture(vec![reply]).await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert_eq!(result.content, "a\u{fffd}");
    let metadata: Retrieval = serde_json::from_value(result.metadata.unwrap()).unwrap();
    assert_eq!(metadata.body_bytes, 2);
    assert_eq!(
        metadata.body_sha256,
        format!("{:x}", Sha256::digest([b'a', 0xff]))
    );
    assert!(metadata.utf8_replacements);
}

#[tokio::test]
async fn a_failed_fresh_request_does_not_return_a_previous_successful_cache() {
    let failure =
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let (_server, url, client) = fixture(vec![response("previous"), failure]).await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let first = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert_eq!(first.content, "previous");
    let second = fetch_page(&params, &context(), &url, &client, directory.path()).await;
    assert!(second.is_error);
    assert!(second.content.starts_with("HTTP 404 "));
    assert!(!second.content.contains("previous"));
}

#[tokio::test]
async fn ambient_proxy_cannot_redirect_the_restricted_client() {
    if std::env::var_os("COVEN_TEST_WEBFETCH_PROXY_WORKER").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "web_fetch::retrieval_tests::ambient_proxy_cannot_redirect_the_restricted_client",
                "--nocapture",
            ])
            .env("COVEN_TEST_WEBFETCH_PROXY_WORKER", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    // This exact-test subprocess owns its environment; no parent test or real
    // proxy setting is mutated. Both common environment spellings are covered.
    for name in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        std::env::set_var(name, &proxy_url);
    }
    for name in ["NO_PROXY", "no_proxy"] {
        std::env::set_var(name, "");
    }
    let (_server, url, client) = fixture(vec![response("direct public fixture")]).await;
    let directory = tempfile::tempdir().unwrap();
    let params = WebFetchInput {
        url: url.to_string(),
        prompt: None,
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        fetch_page(&params, &context(), &url, &client, directory.path()),
    )
    .await
    .expect("public request must not wait for the ambient proxy");
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "direct public fixture");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), proxy.accept())
            .await
            .is_err(),
        "ambient proxy received a connection"
    );
}
