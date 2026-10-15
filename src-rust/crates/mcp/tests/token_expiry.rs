//! Public OAuth response tests use local HTTP and child-only temporary homes.
use claurst_mcp::oauth::{exchange_code, refresh_mcp_token, store_mcp_token, McpToken};
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[test]
fn exchange_rejects_expiry_overflow() {
    run_case("exchange-overflow");
}
#[test]
fn refresh_rejects_expiry_overflow_without_replacing_token() {
    run_case("refresh-overflow");
}
#[test]
fn exchange_preserves_optional_expiry() {
    run_case("exchange-controls");
}
#[test]
fn refresh_preserves_optional_expiry_and_existing_fields() {
    run_case("refresh-controls");
}

fn run_case(case: &str) {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "token_expiry_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("COVEN_TOKEN_EXPIRY_CASE", case)
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{case}: {status}");
            assert!(
                home.path().join("completed").is_file(),
                "fixture must execute"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{case} exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

async fn token_server(body: Value) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(socket);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.starts_with("POST /token HTTP/1.1"));
        let mut content_length = None;
        loop {
            line.clear();
            assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
        }
        let mut request = vec![0; content_length.unwrap()];
        reader.read_exact(&mut request).await.unwrap();
        let body = body.to_string();
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        reader
            .get_mut()
            .write_all(response.as_bytes())
            .await
            .unwrap();
        String::from_utf8(request).unwrap()
    });
    (endpoint, server)
}

#[tokio::test]
#[ignore = "invoked by parent tests with an isolated token home"]
async fn token_expiry_fixture() {
    let case = std::env::var("COVEN_TOKEN_EXPIRY_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let refresh = case.starts_with("refresh");
    let overflow = case.ends_with("overflow");
    let expirations = if overflow {
        vec![Some(u64::MAX)]
    } else {
        vec![None, Some(0)]
    };
    for expires_in in expirations {
        let mut body = json!({"access_token": "new-access"});
        if let Some(seconds) = expires_in {
            body["expires_in"] = json!(seconds);
        }
        // Refresh has always ignored this field, even if its type is not a string.
        body["scope"] = if refresh {
            json!({"ignored": true})
        } else {
            json!("new-scope")
        };
        let (endpoint, server) = token_server(body).await;
        let original = McpToken {
            access_token: "old-access".into(),
            refresh_token: Some("old-refresh".into()),
            expires_at: Some(1),
            scope: Some("old-scope".into()),
            server_name: "expiry-fixture".into(),
            server_url: Some("https://fixture.invalid/mcp".into()),
        };
        let token_path = home.join("mcp-tokens/expiry-fixture.json");
        let before = if refresh {
            store_mcp_token(&original).unwrap();
            Some(std::fs::read(&token_path).unwrap())
        } else {
            None
        };
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let result = if refresh {
            refresh_mcp_token("expiry-fixture", "https://fixture.invalid/mcp", &endpoint).await
        } else {
            exchange_code(
                &endpoint,
                "dummy-code",
                "dummy-verifier",
                "http://127.0.0.1/callback",
            )
            .await
        };
        let request = server.await.unwrap();
        assert!(request.contains(if refresh {
            "grant_type=refresh_token"
        } else {
            "grant_type=authorization_code"
        }));
        if overflow {
            let error = result.expect_err("overflow must be rejected").to_string();
            assert!(error.contains("expires_in"), "{error}");
            if let Some(before) = before {
                assert_eq!(std::fs::read(token_path).unwrap(), before);
            }
        } else {
            let token = result.unwrap();
            assert_eq!(token.access_token, "new-access");
            if expires_in.is_none() {
                assert_eq!(token.expires_at, None);
            } else {
                let finished = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                assert!((started..=finished).contains(&token.expires_at.unwrap()));
            }
            if refresh {
                assert_eq!(token.refresh_token.as_deref(), Some("old-refresh"));
                assert_eq!(token.scope.as_deref(), Some("old-scope"));
                assert_eq!(token.server_name, "expiry-fixture");
                assert_eq!(token.server_url, original.server_url);
                let stored: McpToken =
                    serde_json::from_slice(&std::fs::read(token_path).unwrap()).unwrap();
                assert_eq!(
                    serde_json::to_value(stored).unwrap(),
                    serde_json::to_value(token).unwrap()
                );
            } else {
                assert_eq!(token.refresh_token, None);
                assert_eq!(token.scope.as_deref(), Some("new-scope"));
                assert!(token.server_name.is_empty());
                assert_eq!(token.server_url, None);
                assert!(!home.join("mcp-tokens").exists());
            }
        }
    }
    std::fs::write(home.join("completed"), b"complete").unwrap();
}
