//! `COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC` turns off the model catalog
//! refresh like `COVEN_CODE_DISABLE_MODELS_FETCH` does (follow-up to #516).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A catalog server that answers every request with a 500 and counts them.
fn counting_catalog() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api.json", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            counted.fetch_add(1, Ordering::SeqCst);
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    (url, hits)
}

/// Runs `coven-code models --refresh --json` against `url` in a scratch home,
/// with `extra` added to the environment, and returns its stderr.
fn refresh_models(url: &str, extra: &[(&str, &str)]) -> String {
    let home = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_coven-code"));
    command
        .args(["models", "--refresh", "--json"])
        .env_remove("COVEN_CODE_DISABLE_MODELS_FETCH")
        .env_remove("COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC")
        .env_remove("MODELS_DEV_URL")
        .env("COVEN_CODE_MODELS_URL", url)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("COVEN_CODE_HOME", home.path().join("code"))
        .env("COVEN_HOME", home.path().join("coven"))
        .stdin(std::process::Stdio::null());
    for (key, value) in extra {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_nonessential_traffic_variable_turns_off_the_model_catalog_refresh() {
    // Control: without an opt-out, the refresh reaches the server.
    let (url, hits) = counting_catalog();
    let stderr = refresh_models(&url, &[]);
    assert_eq!(hits.load(Ordering::SeqCst), 1, "{stderr}");

    // Presence is enough, as for the other opt-outs.
    for value in ["1", "0", ""] {
        let (url, hits) = counting_catalog();
        let stderr = refresh_models(&url, &[("COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC", value)]);
        assert_eq!(hits.load(Ordering::SeqCst), 0, "value {value:?}: {stderr}");
    }
}
