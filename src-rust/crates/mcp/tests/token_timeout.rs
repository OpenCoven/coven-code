//! Exercise the production deadline against owned loopback peers in isolated homes.
use claurst_mcp::oauth::{exchange_code, refresh_mcp_token, store_mcp_token, McpToken};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

fn run_case(case: &str) {
    let home = tempfile::tempdir().unwrap();
    let log_path = home.path().join("worker.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "timeout_worker", "--ignored", "--nocapture"])
        .env("COVEN_TOKEN_TIMEOUT_CASE", case)
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1")
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(50);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{case} worker exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let log = std::fs::read_to_string(log_path).unwrap();
    assert!(status.success(), "{case}: {status}\n{log}");
    assert!(home.path().join("completed").is_file(), "{log}");
}

#[test]
fn exchange_header_timeout() {
    run_case("exchange-headers");
}
#[test]
fn exchange_body_timeout() {
    run_case("exchange-body");
}
#[test]
fn exchange_error_body_timeout() {
    run_case("exchange-error-body");
}
#[test]
fn refresh_header_timeout() {
    run_case("refresh-headers");
}
#[test]
fn refresh_body_timeout() {
    run_case("refresh-body");
}
#[test]
fn refresh_error_body_timeout() {
    run_case("refresh-error-body");
}
#[test]
fn exchange_response_controls() {
    run_case("exchange-controls");
}
#[test]
fn refresh_response_controls() {
    run_case("refresh-controls");
}

#[tokio::test]
#[ignore = "owned temporary-home worker explicitly invoked by parent"]
async fn timeout_worker() {
    let case = std::env::var("COVEN_TOKEN_TIMEOUT_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let refresh = case.starts_with("refresh");
    let modes: &[&str] = if case.ends_with("controls") {
        &["success", "http-error"]
    } else if case.ends_with("error-body") {
        &["error-body"]
    } else if case.ends_with("headers") {
        &["headers"]
    } else {
        &["body"]
    };
    for &mode in modes {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let (observed_tx, observed_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(socket);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert!(line.starts_with("POST /token HTTP/1.1"));
            let mut length = None;
            loop {
                line.clear();
                assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = Some(value.trim().parse::<usize>().unwrap());
                    }
                }
            }
            let mut request = vec![0; length.unwrap()];
            reader.read_exact(&mut request).await.unwrap();
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains(if refresh {
                "grant_type=refresh_token"
            } else {
                "grant_type=authorization_code"
            }));
            if mode != "headers" {
                let status = if mode == "error-body" || mode == "http-error" {
                    "400 Bad Request"
                } else {
                    "200 OK"
                };
                let body = if mode == "success" {
                    r#"{"access_token":"new-access","refresh_token":"new-refresh"}"#
                } else {
                    "denied"
                };
                let length = if mode == "success" || mode == "http-error" {
                    body.len()
                } else {
                    body.len() + 100
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}");
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
            observed_tx.send(()).unwrap();
            if mode != "success" && mode != "http-error" {
                // Keep the connection open until the parent aborts this owned task.
                std::future::pending::<()>().await;
            }
            drop(reader);
        });
        let original = McpToken {
            access_token: "old-access".into(),
            refresh_token: Some("old-refresh".into()),
            expires_at: None,
            scope: Some("old-scope".into()),
            server_name: "timeout-fixture".into(),
            server_url: Some("https://fixture.invalid/mcp".into()),
        };
        let token_path = home.join("mcp-tokens/timeout-fixture.json");
        let before = if refresh {
            store_mcp_token(&original).unwrap();
            Some(std::fs::read(&token_path).unwrap())
        } else {
            None
        };
        let started = Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(40), async {
            if refresh {
                refresh_mcp_token("timeout-fixture", "https://fixture.invalid/mcp", &endpoint).await
            } else {
                exchange_code(
                    &endpoint,
                    "dummy-code",
                    "dummy-verifier",
                    "http://127.0.0.1/callback",
                )
                .await
            }
        })
        .await;
        tokio::time::timeout(Duration::from_secs(1), observed_rx)
            .await
            .unwrap()
            .unwrap();
        if mode == "success" || mode == "http-error" {
            server.await.unwrap();
        } else {
            server.abort();
            assert!(server.await.unwrap_err().is_cancelled());
        }
        let result =
            result.expect("token request exceeded outer 40s guard without its own deadline");
        if mode == "success" {
            let token = result.unwrap();
            assert_eq!(token.access_token, "new-access");
            if refresh {
                let stored: McpToken =
                    serde_json::from_slice(&std::fs::read(&token_path).unwrap()).unwrap();
                assert_eq!(stored.access_token, "new-access");
                assert_eq!(stored.refresh_token.as_deref(), Some("new-refresh"));
                assert_eq!(stored.scope.as_deref(), Some("old-scope"));
            }
        } else {
            let error = result.unwrap_err();
            if mode == "http-error" || mode == "error-body" {
                // Error bodies are never read or echoed (token responses can
                // reflect credentials), so a stalled error body cannot hold
                // the request open either.
                assert!(error.to_string().contains("HTTP 400"), "{error:#}");
                assert!(!format!("{error:#}").contains("denied"), "{error:#}");
                assert!(started.elapsed() < Duration::from_secs(25), "{error:#}");
            } else {
                let request_error = error
                    .downcast_ref::<reqwest::Error>()
                    .expect("typed request error retained");
                assert!(request_error.is_timeout(), "{error:#}");
                assert!(started.elapsed() >= Duration::from_secs(25));
                assert!(started.elapsed() < Duration::from_secs(40));
                assert!(
                    error.to_string().contains("timed out after 30 seconds"),
                    "{error:#}"
                );
                assert!(
                    error.to_string().contains(if refresh {
                        "refresh:"
                    } else {
                        "exchange_code:"
                    }),
                    "{error:#}"
                );
            }
            if let Some(before) = before {
                assert_eq!(std::fs::read(&token_path).unwrap(), before);
            }
        }
        if !refresh {
            assert!(!token_path.exists());
        }
    }
    std::fs::write(home.join("completed"), b"complete").unwrap();
}
