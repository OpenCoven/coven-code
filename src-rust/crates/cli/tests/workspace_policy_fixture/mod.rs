//! Shared by the workspace policy tests: a host policy and a loopback stand-in
//! for the Messages API, so a scoped run needs no provider or credentials.
//! Each test crate that includes this module builds on a different platform,
//! and each must use every helper here, or `dead_code` fails clippy.

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    sync::mpsc,
    thread,
};

pub fn scope(run_id: &str, work: &Path, writable: &[&Path], read_only: &[&Path]) -> Value {
    json!({
        "version": 1, "runId": run_id, "access": "workspace-write",
        "primaryRoot": work, "writableRoots": writable, "readOnlyRoots": read_only,
    })
}

/// A loopback stand-in for the Messages API. Each connection is answered
/// with the next scripted reply, and its request line is sent to the test.
/// A thread whose replies a correct run never asks for stays blocked in
/// `accept` until the test binary exits.
pub fn messages_fixture(replies: Vec<String>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (requests, received) = mpsc::channel();
    // A stopped run may drop its connection part way; the thread then ends.
    thread::spawn(move || {
        for reply in replies {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            let Ok(clone) = socket.try_clone() else {
                return;
            };
            let mut reader = BufReader::new(clone);
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                return;
            }
            let mut length = 0;
            loop {
                let mut header = String::new();
                match reader.read_line(&mut header) {
                    Ok(0) | Err(_) => return,
                    Ok(_) if header == "\r\n" => break,
                    Ok(_) => {}
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        let Ok(value) = value.trim().parse() else {
                            return;
                        };
                        length = value;
                    }
                }
            }
            if reader.read_exact(&mut vec![0; length]).is_err() {
                return;
            }
            let _ = requests.send(request);
            let _ = write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    (base, received)
}

/// One streamed model reply holding a single content block, in the wire
/// shape of the Messages API.
pub fn model_reply(block: Value, delta: Value, stop_reason: &str) -> String {
    [
        json!({"type": "message_start", "message": {
            "id": "fixture", "type": "message", "role": "assistant", "content": [],
            "model": "fixture", "usage": {"input_tokens": 1, "output_tokens": 0},
        }}),
        json!({"type": "content_block_start", "index": 0, "content_block": block}),
        json!({"type": "content_block_delta", "index": 0, "delta": delta}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": stop_reason},
            "usage": {"output_tokens": 1}}),
        json!({"type": "message_stop"}),
    ]
    .into_iter()
    .map(|data| {
        format!(
            "event: {}\ndata: {data}\n\n",
            data["type"].as_str().unwrap()
        )
    })
    .collect()
}

pub fn final_text(text: &str) -> String {
    model_reply(
        json!({"type": "text", "text": ""}),
        json!({"type": "text_delta", "text": text}),
        "end_turn",
    )
}

pub fn user_frame(text: &str) -> Vec<u8> {
    format!(
        "{}\n",
        json!({"type": "user", "message": {"role": "user", "content": text}})
    )
    .into_bytes()
}
