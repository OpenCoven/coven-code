use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::voice::VoiceEvent;
use claurst_tui::application::UiRenderer;
use claurst_tui::{App, NotificationKind};
use ratatui::{backend::TestBackend, Terminal};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

const SECRET: &str = "synthetic-tui-transcription-secret";
// The final character crosses byte 60 while remaining the 60th character.
const UNICODE_TRANSCRIPT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaé";

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
            let unicode_body = format!(r#"{{"text":"{UNICODE_TRANSCRIPT}"}}"#);
            let (status, body) = match case.as_str() {
                "http" => ("403 Forbidden", SECRET),
                "decode" => ("200 OK", SECRET),
                "missing" => ("200 OK", "{}"),
                "unicode" => ("200 OK", unicode_body.as_str()),
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
        .env("COVEN_CAVE_HOME", home.path().join("cave"))
        .env("COVEN_CODE_VOICE", "0")
        .env("WHISPER_ENDPOINT_URL", endpoint)
        .env("OWNED_VOICE_CASE", case)
        .output()
        .unwrap();
    if let Some(server) = server {
        server.join().unwrap();
    }
    // Never echo child output: a privacy regression must not copy its canary
    // or credential-bearing endpoint into assertion diagnostics.
    assert!(
        output.status.success(),
        "isolated voice polling case failed ({case})"
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
fn tui_unicode_success_preserves_draft_and_clears_receiver() {
    run_case("unicode");
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
    let event = match case.as_str() {
        "success" | "unicode" | "missing" => {
            assert!(result.is_ok(), "transcription unexpectedly failed");
            let text = result.unwrap();
            let expected = match case.as_str() {
                "success" => "synthetic transcript",
                "unicode" => UNICODE_TRANSCRIPT,
                _ => "",
            };
            assert!(text == expected, "transcript content changed");
            VoiceEvent::TranscriptReady(text)
        }
        _ => {
            assert!(result.is_err(), "transcription unexpectedly succeeded");
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
                assert!(!text.contains(SECRET), "private value escaped");
                assert!(!text.contains("127.0.0.1"), "endpoint escaped");
                assert!(text.contains(category), "missing error category");
            }
            assert_eq!(error.chain().count(), 1);
            // Match the recorder's production error-to-event boundary using
            // the error from the actual local HTTP transcription call.
            VoiceEvent::Error(error.to_string())
        }
    };
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        for (width, height) in [(80, 24), (120, 40)] {
            poll_and_render(&case, event.clone(), renderer, width, height);
        }
    }
}

fn poll_and_render(case: &str, event: VoiceEvent, renderer: UiRenderer, width: u16, height: u16) {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.select_renderer(renderer);
    app.has_credentials = true;
    app.set_prompt_text("typed draft".into());
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    app.voice_event_rx = Some(rx);

    // Exercise empty polls and recording state, then drain a producer result
    // together with RecordingStopped in the order sent by the recorder.
    app.poll_voice_events();
    assert!(app.voice_event_rx.is_some(), "idle poll dropped receiver");
    tx.try_send(VoiceEvent::RecordingStarted).unwrap();
    app.poll_voice_events();
    assert!(app.voice_recording, "recording did not start");
    assert!(app.status_message.as_deref() == Some("Recording… (Alt+V or Esc to stop)"));
    tx.try_send(VoiceEvent::RecordingStopped).unwrap();
    tx.try_send(event).unwrap();
    app.poll_voice_events();
    assert!(!app.voice_recording, "recording state was not cleared");
    assert!(
        app.voice_event_rx.is_none(),
        "result receiver was not cleared"
    );
    match case {
        "success" => {
            assert!(
                app.prompt_input.text == "typed draft synthetic transcript",
                "transcript insertion changed"
            );
            assert!(app.status_message.as_deref() == Some("Transcribed: synthetic transcript"));
            assert!(
                app.notifications.is_empty(),
                "success created a notification"
            );
        }
        "unicode" => {
            assert!(
                app.prompt_input.text == format!("typed draft {UNICODE_TRANSCRIPT}"),
                "Unicode transcript insertion changed"
            );
            assert!(
                app.status_message.as_deref()
                    == Some(format!("Transcribed: {UNICODE_TRANSCRIPT}").as_str()),
                "Unicode transcript preview changed"
            );
            assert!(
                app.notifications.is_empty(),
                "Unicode success created a notification"
            );
        }
        "missing" => {
            assert!(
                app.prompt_input.text == "typed draft",
                "empty result changed draft"
            );
            assert!(app.status_message.as_deref() == Some("Transcribing…"));
            assert!(
                app.notifications.is_empty(),
                "empty result created a notification"
            );
        }
        _ => {
            assert!(
                app.prompt_input.text == "typed draft",
                "error changed draft"
            );
            assert!(app.status_message.as_deref() == Some("Transcribing…"));
            let notification = app.notifications.current().expect("voice warning missing");
            assert!(
                notification.kind == NotificationKind::Warning,
                "voice error severity changed"
            );
            assert!(
                notification.message.starts_with("Voice: "),
                "voice context missing"
            );
            assert!(
                notification.message.contains("Transcription"),
                "transcription context missing"
            );
            assert!(
                !notification.message.contains(SECRET),
                "voice warning exposed a private value"
            );
            assert!(notification.expires_at.is_some(), "warning lost expiration");
        }
    }
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| match renderer {
            UiRenderer::Classic => claurst_tui::render::render_app(frame, &app),
            UiRenderer::Next => claurst_tui::next::render_next(frame, &app),
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        !rendered.contains(SECRET),
        "rendered frame exposed a private value"
    );
    assert!(
        !rendered.contains("127.0.0.1"),
        "rendered frame exposed endpoint"
    );
    assert!(
        rendered.contains("typed draft"),
        "rendered frame lost draft"
    );
    match case {
        "success" => assert!(
            rendered.contains("synthetic transcript"),
            "rendered frame lost transcript"
        ),
        "unicode" => assert!(
            rendered.contains('é'),
            "rendered frame lost Unicode transcript"
        ),
        "missing" => {}
        _ => {
            let category = match case {
                "http" => "403",
                "decode" => "decode",
                _ => "connection",
            };
            assert!(
                rendered.contains(category),
                "rendered warning lost error category ({renderer:?} {width}x{height})"
            );
            assert!(
                rendered.contains("Voice:"),
                "rendered warning lost voice context"
            );
            let warning_marker = match renderer {
                UiRenderer::Classic => NotificationKind::Warning.icon(),
                UiRenderer::Next => "warning",
            };
            assert!(
                rendered.contains(warning_marker),
                "rendered warning lost severity"
            );
        }
    }
    app.poll_voice_events();
    assert!(
        app.voice_event_rx.is_none(),
        "finished polling restored receiver"
    );
}
