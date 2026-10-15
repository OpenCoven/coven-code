#![cfg(any(target_os = "macos", target_os = "linux"))]

use claurst_mcp::oauth::{initiate_xaa_login, pkce_challenge};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_case(case: &str, launcher_status: &str) {
    let temp = tempfile::tempdir().unwrap();
    let launcher = temp.path().join(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    });
    // No PATH fallback or external shell commands: the real browser cannot run.
    std::fs::write(
        &launcher,
        "#!/bin/sh\n[ \"$#\" -eq 1 ] || exit 91\nprintf '%s' \"$1\" > \"$XAA_CAPTURE\"\nexit \"$XAA_LAUNCHER_STATUS\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output_path = temp.path().join("worker.log");
    let output = std::fs::File::create(&output_path).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "xaa_worker", "--ignored", "--nocapture"])
        .env_clear()
        .env("PATH", temp.path())
        .env("XAA_CAPTURE", temp.path().join("url"))
        .env("XAA_CASE", case)
        .env("XAA_LAUNCHER_STATUS", launcher_status)
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("XAA fixture timed out: {case}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let log = std::fs::read_to_string(output_path).unwrap();
    assert!(status.success(), "case {case}: {status}\n{log}");
    assert!(log.contains("XAA fixture completed"), "{log}");
}

#[test]
fn xaa_login_preserves_endpoint_query_and_returned_state() {
    for case in ["plain", "query", "fragment", "query_fragment"] {
        run_case(case, "0");
    }
    run_case("query", "1"); // Browser launch remains best-effort.
}

#[test]
fn xaa_login_rejects_invalid_url_before_launch() {
    run_case("invalid", "0");
}

#[test]
#[ignore = "isolated environment worker invoked by parent tests"]
fn xaa_worker() {
    let case = std::env::var("XAA_CASE").unwrap();
    let capture = std::path::PathBuf::from(std::env::var_os("XAA_CAPTURE").unwrap());
    let endpoint = match case.as_str() {
        "plain" => "https://example.invalid/authorize",
        "query" => "https://example.invalid/authorize?audience=mcp%2Btools",
        "fragment" => "https://example.invalid/authorize#login",
        "query_fragment" => "https://example.invalid/authorize?audience=mcp%2Btools#login",
        "invalid" => "not an absolute URL",
        _ => panic!("unknown fixture case"),
    };
    if case == "invalid" {
        let error = initiate_xaa_login("fixture", endpoint).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error
            .get_ref()
            .unwrap()
            .downcast_ref::<url::ParseError>()
            .is_some());
        assert!(!capture.exists(), "invalid URL reached browser launcher");
    } else {
        let state = initiate_xaa_login("fixture", endpoint).unwrap();
        assert_eq!(state.server_name, "fixture");
        assert_eq!(state.idp_url, endpoint);
        assert_ne!(state.redirect_port, 0);
        let captured = std::fs::read_to_string(capture).unwrap();
        let url = url::Url::parse(&captured).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("example.invalid"));
        assert_eq!(url.path(), "/authorize");
        assert_eq!(
            url.query_pairs().count(),
            if case.contains("query") { 5 } else { 4 }
        );
        let query: std::collections::BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(
            query.get("code_challenge"),
            Some(&pkce_challenge(&state.verifier))
        );
        assert_eq!(
            query.get("redirect_uri"),
            Some(&format!(
                "http://127.0.0.1:{}/callback",
                state.redirect_port
            ))
        );
        if case.contains("query") {
            assert_eq!(query.get("audience").map(String::as_str), Some("mcp+tools"));
        }
        assert_eq!(
            url.fragment(),
            if case.contains("fragment") {
                Some("login")
            } else {
                None
            }
        );
    }
    println!("XAA fixture completed");
}
