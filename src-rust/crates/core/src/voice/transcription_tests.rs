use super::transcribe_audio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};

const CANARY: &str = "private-transcription-canary";

async fn read_request(socket: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let count = socket.read(&mut chunk).await?;
        anyhow::ensure!(count > 0, "fixture request ended early");
        request.extend_from_slice(&chunk[..count]);
        anyhow::ensure!(request.len() < 65536, "fixture request exceeds bound");
        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let header = std::str::from_utf8(&request[..end])?;
            let length: usize = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then_some(value.trim())
                })
                .unwrap_or("0")
                .parse()?;
            if request.len() >= end + 4 + length {
                return Ok(request);
            }
        }
    }
}

// Exercise the production HTTP boundary without a provider, microphone or
// ambient credentials. Every socket operation has a deadline and the server
// task is joined even when the privacy assertion fails.
async fn response(
    status: &str,
    headers: &str,
    body: &str,
    length: usize,
) -> anyhow::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n{headers}\r\n{body}"
    );
    let server = tokio::spawn(async move {
        timeout(Duration::from_secs(5), async {
            let (mut socket, _) = listener.accept().await?;
            let request = read_request(&mut socket).await?;
            // Multipart contains binary WAV bytes; only inspect its ASCII
            // target, headers and field markers as text.
            let request = String::from_utf8_lossy(&request);
            anyhow::ensure!(
                request.starts_with(&format!("POST /{CANARY}?token={CANARY} HTTP/1.1\r\n")),
                "request target changed"
            );
            anyhow::ensure!(
                request
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {CANARY}\r\n")),
                "authorization changed"
            );
            anyhow::ensure!(
                request.contains("audio.wav") && request.contains("whisper-1"),
                "multipart changed"
            );
            socket.write_all(reply.as_bytes()).await?;
            socket.shutdown().await?;
            Ok::<_, anyhow::Error>(())
        })
        .await?
    });
    let endpoint = format!("http://{address}/{CANARY}?token={CANARY}#{CANARY}");
    let result = timeout(
        Duration::from_secs(5),
        transcribe_audio(
            &[0.0; 16],
            16000,
            CANARY,
            Some("en"),
            "whisper-1",
            Some(&endpoint),
        ),
    )
    .await;
    server.await??;
    result?
}

fn private_error(error: anyhow::Error, category: &str) {
    for formatted in [
        error.to_string(),
        format!("{error:#}"),
        format!("{error:?}"),
    ] {
        assert!(
            !formatted.contains(CANARY),
            "private data escaped: {formatted}"
        );
        assert!(
            !formatted.contains("127.0.0.1"),
            "endpoint escaped: {formatted}"
        );
        assert!(
            formatted.contains(category),
            "missing category {category}: {formatted}"
        );
    }
    assert_eq!(error.chain().count(), 1, "raw transport source retained");
}

#[tokio::test]
async fn http_error_keeps_status_without_response_body() {
    let error = response("401 Unauthorized", "", CANARY, CANARY.len())
        .await
        .unwrap_err();
    private_error(error, "401");
}

#[tokio::test]
async fn malformed_json_omits_endpoint_and_body() {
    let error = response("200 OK", "", CANARY, CANARY.len())
        .await
        .unwrap_err();
    private_error(error, "decode");
}

#[tokio::test]
async fn incomplete_response_omits_endpoint() {
    let error = response("200 OK", "", "{", 1000).await.unwrap_err();
    // reqwest classifies response.json() body failures as decode failures.
    private_error(error, "decode");
}

#[tokio::test]
async fn unsupported_redirect_omits_destination_and_keeps_status() {
    let headers = format!("Location: file:///{CANARY}\r\n");
    let error = response("302 Found", &headers, "", 0).await.unwrap_err();
    // reqwest leaves unsupported redirect schemes as the original response.
    private_error(error, "302");
}

#[tokio::test]
async fn redirect_loop_is_a_private_redirect_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/{CANARY}?token={CANARY}",
        listener.local_addr().unwrap()
    );
    // 302 switches the multipart POST to GET. A 307 cannot replay reqwest's
    // streaming multipart body and would stop before exercising the limit.
    let reply = format!("HTTP/1.1 302 Found\r\nLocation: {endpoint}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        timeout(Duration::from_secs(5), async {
            let mut requests = 0;
            loop {
                let (mut socket, _) = tokio::select! {
                    biased;
                    _ = &mut stopped => return Ok::<_, anyhow::Error>(requests),
                    connection = listener.accept() => connection?,
                };
                read_request(&mut socket).await?;
                socket.write_all(reply.as_bytes()).await?;
                socket.shutdown().await?;
                requests += 1;
            }
        })
        .await?
    });
    let result = timeout(
        Duration::from_secs(5),
        transcribe_audio(&[], 16000, CANARY, None, "whisper-1", Some(&endpoint)),
    )
    .await;
    let _ = stop.send(());
    assert!(
        server.await.unwrap().unwrap() > 1,
        "fixture must follow redirects"
    );
    private_error(result.unwrap().unwrap_err(), "redirect");
}

#[tokio::test]
async fn refused_connection_omits_userinfo_query_and_fragment() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{CANARY}:{CANARY}@{}/{CANARY}?token={CANARY}#{CANARY}",
        listener.local_addr().unwrap()
    );
    drop(listener);
    let error = timeout(
        Duration::from_secs(5),
        transcribe_audio(&[], 16000, CANARY, None, "whisper-1", Some(&endpoint)),
    )
    .await
    .unwrap()
    .unwrap_err();
    private_error(error, "connection");
}

#[tokio::test]
async fn invalid_endpoint_is_a_private_builder_failure() {
    let endpoint = format!("http://[invalid-{CANARY}");
    let error = transcribe_audio(&[], 16000, CANARY, None, "whisper-1", Some(&endpoint))
        .await
        .unwrap_err();
    private_error(error, "builder");
}

#[tokio::test]
async fn successful_transcript_stays_trimmed() {
    let body = r#"{"text":"  synthetic transcript  "}"#;
    assert_eq!(
        response("200 OK", "", body, body.len()).await.unwrap(),
        "synthetic transcript"
    );
}

#[tokio::test]
async fn missing_text_retains_existing_empty_transcript_behavior() {
    assert_eq!(response("200 OK", "", "{}", 2).await.unwrap(), "");
}
