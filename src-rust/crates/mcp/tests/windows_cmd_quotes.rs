//! #517 on Windows: the XAA login opens its URL through `cmd.exe /C start`,
//! which must get the whole URL. Before, the URL reached `cmd.exe` unquoted,
//! and `cmd.exe` ended the command at the URL's first `&`.
//!
//! `cmd.exe` also replaces `%NAME%` with the variable's value, which can be
//! a `"` that ends the quoting around the URL.
//!
//! A URL scheme registered for the current user stands in for the browser:
//! Windows runs this test binary with the URL it was asked to open. Whether a
//! real browser then shows the page can only be seen on a desktop.
#![cfg(windows)]

use claurst_mcp::oauth::{initiate_xaa_login, pkce_challenge};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const CAPTURE: &str = "capture";

// The registry helpers are repeated in claurst-tui's `windows_open_tests`:
// test crates cannot share a module.

/// A registry key under `HKCU\Software\Classes`, removed on drop.
struct Registered(String);

impl Drop for Registered {
    fn drop(&mut self) {
        let _ = Command::new("reg")
            .args(["delete", self.0.as_str(), "/f"])
            .output();
    }
}

fn reg_add(key: &str, value: &[&str], data: &str) {
    let output = Command::new("reg")
        .arg("add")
        .arg(key)
        .args(value)
        .args(["/d", data, "/f"])
        .output()
        .unwrap();
    assert!(output.status.success(), "reg add {key}: {output:?}");
}

/// Make `scheme:` URLs open with this test binary, which writes the URL it
/// was given to `capture`.
fn register_scheme(scheme: &str, capture: &Path) -> Registered {
    let registered = Registered(format!(r"HKCU\Software\Classes\{scheme}"));
    reg_add(&registered.0, &["/ve"], "URL:coven-code #517 test");
    reg_add(&registered.0, &["/v", "URL Protocol"], "");
    let handler = format!(
        r#""{}" --exact url_open_handler --ignored --nocapture "{}" "%1""#,
        std::env::current_exe().unwrap().display(),
        capture.display(),
    );
    reg_add(
        &format!(r"{}\shell\open\command", registered.0),
        &["/ve"],
        &handler,
    );
    registered
}

fn wait_for(capture: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(text) = std::fs::read_to_string(capture) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "nothing was opened within 60 s: {} was not written",
            capture.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn xaa_login_opens_the_whole_url() {
    let directory = tempfile::tempdir().unwrap();
    let capture = directory.path().join(format!("url.{CAPTURE}"));
    let scheme = format!("coven-test-517-{}", std::process::id());
    let _registered = register_scheme(&scheme, &capture);
    // The login appends four parameters with `&`. `%PATH%` is set, and
    // `cmd.exe` would replace it with its value.
    let endpoint = format!("{scheme}://authorize?audience=mcp%2Btools&note=%PATH%");

    // The launch waits for `cmd.exe`; bound it, should Windows ask anything.
    let (sent, launched) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sent.send(initiate_xaa_login("fixture", &endpoint));
    });
    let state = launched
        .recv_timeout(Duration::from_secs(60))
        .expect("the login did not return within 60 s")
        .unwrap();

    let opened = wait_for(&capture);
    let url = url::Url::parse(&opened).unwrap();
    assert_eq!(url.scheme(), scheme, "{opened}");
    let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    let expected = BTreeMap::from([
        ("audience".to_string(), "mcp+tools".to_string()),
        ("note".to_string(), "%PATH%".to_string()),
        ("response_type".to_string(), "code".to_string()),
        (
            "code_challenge".to_string(),
            pkce_challenge(&state.verifier),
        ),
        ("code_challenge_method".to_string(), "S256".to_string()),
        (
            "redirect_uri".to_string(),
            format!("http://127.0.0.1:{}/callback", state.redirect_port),
        ),
    ]);
    assert_eq!(query, expected, "Windows was asked to open {opened}");
}

/// Run by Windows as the test scheme's handler, with the capture file and
/// the URL as its last two arguments. It does nothing unless the first of
/// those is a capture file.
#[test]
#[ignore = "run by Windows as the handler of the test URL scheme"]
fn url_open_handler() {
    let mut arguments = std::env::args_os().rev();
    let (Some(opened), Some(capture)) = (arguments.next(), arguments.next()) else {
        return;
    };
    let capture = PathBuf::from(capture);
    if capture
        .extension()
        .is_none_or(|extension| extension != CAPTURE)
    {
        return;
    }
    let partial = capture.with_extension("partial");
    std::fs::write(&partial, opened.to_string_lossy().as_bytes()).unwrap();
    std::fs::rename(partial, capture).unwrap();
}
