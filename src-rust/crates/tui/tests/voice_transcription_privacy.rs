use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

const SECRET: &str = "synthetic-tui-transcription-secret";

fn run_case(case: &str) {
    let home = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let endpoint = format!("http://{SECRET}:{SECRET}@{address}/{SECRET}?token={SECRET}#{SECRET}");
    let server = if case == "connection" {
        drop(listener);
        None
    } else {
        listener.set_nonblocking(true).unwrap();
        let case = case.to_owned();
        Some(std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            // Accepted sockets can inherit the nonblocking listener flag on
            // macOS. This fixture deliberately uses blocking bounded reads.
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let count = socket.read(&mut chunk).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
                assert!(request.len() < 65536);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then_some(value.trim())
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&request);
            assert!(text.starts_with(&format!("POST /{SECRET}?token={SECRET} HTTP/1.1")));
            assert!(text
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {SECRET}")));
            assert!(
                text.contains("audio.wav") && text.contains("whisper-1") && text.contains("RIFF")
            );
            let (status, body) = match case.as_str() {
                "http" => ("403 Forbidden", SECRET),
                "decode" => ("200 OK", SECRET),
                "missing" => ("200 OK", "{}"),
                _ => ("200 OK", "{\"text\":\"  synthetic transcript  \"}"),
            };
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }))
    };
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "voice_privacy_worker",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home.path())
        .env("COVEN_CODE_HOME", home.path().join("engine"))
        .env("COVEN_HOME", home.path().join("coven"))
        .env("WHISPER_ENDPOINT_URL", endpoint)
        .env("OWNED_VOICE_CASE", case)
        .output()
        .unwrap();
    if let Some(server) = server {
        server.join().unwrap();
    }
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn tui_http_error_omits_body() {
    run_case("http");
}
#[test]
fn tui_decode_error_omits_endpoint() {
    run_case("decode");
}
#[test]
fn tui_connection_error_omits_endpoint() {
    run_case("connection");
}
#[test]
fn tui_success_preserves_request_and_trimmed_text() {
    run_case("success");
}
#[test]
fn tui_missing_text_preserves_empty_result() {
    run_case("missing");
}

#[tokio::test]
#[ignore = "isolated environment worker invoked by parent tests"]
async fn voice_privacy_worker() {
    let case = std::env::var("OWNED_VOICE_CASE").unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        claurst_tui::voice_capture::transcribe(
            claurst_tui::voice_capture::samples_to_wav_bytes(&[0.0; 16], 16000),
            SECRET,
        ),
    )
    .await
    .unwrap();
    match case.as_str() {
        "success" => assert_eq!(result.unwrap(), "synthetic transcript"),
        "missing" => assert_eq!(result.unwrap(), ""),
        _ => {
            let error = result.unwrap_err();
            let category = match case.as_str() {
                "http" => "403",
                "decode" => "decode",
                _ => "connection",
            };
            for text in [
                error.to_string(),
                format!("{error:#}"),
                format!("{error:?}"),
            ] {
                assert!(!text.contains(SECRET), "private value escaped: {text}");
                assert!(!text.contains("127.0.0.1"), "endpoint escaped: {text}");
                assert!(text.contains(category), "missing category: {text}");
            }
            assert_eq!(error.chain().count(), 1);
        }
    }
}
