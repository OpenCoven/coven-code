//! Token storage permission checks run with a child-only engine home.

use claurst_mcp::oauth::{get_mcp_token, remove_mcp_token, store_mcp_token, McpToken};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn isolated_home(test_name: &str) -> bool {
    const CHILD: &str = "COVEN_CODE_MCP_PERMISSION_TEST_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(test_name) {
        return true;
    }
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name])
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .env(CHILD, test_name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("test result: ok. 1 passed;"), "{stdout}");
    false
}

fn fixture_token(name: &str) -> McpToken {
    McpToken {
        access_token: "fixture-access".into(),
        refresh_token: Some("fixture-refresh".into()),
        expires_at: Some(1_700_000_000),
        scope: Some("fixture-scope".into()),
        server_name: name.into(),
        server_url: Some("https://example.invalid/mcp".into()),
    }
}

#[cfg(unix)]
#[test]
fn token_store_restricts_permissions() {
    if !isolated_home("token_store_restricts_permissions") {
        return;
    }

    let directory = claurst_core::config::config_home().join("mcp-tokens");
    let path = directory.join("permission-fixture.json");
    let mut token = fixture_token("permission-fixture");
    store_mcp_token(&token).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        serde_json::to_vec_pretty(&token).unwrap()
    );
    let new_file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    let new_dir_mode = std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777;

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    token.access_token = "updated-fixture-access".into();
    store_mcp_token(&token).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        serde_json::to_vec_pretty(&token).unwrap()
    );
    let restored = get_mcp_token(&token.server_name).unwrap();
    assert_eq!(
        serde_json::to_value(restored).unwrap(),
        serde_json::to_value(token).unwrap()
    );
    let existing_file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    let existing_dir_mode = std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        (
            new_file_mode,
            new_dir_mode,
            existing_file_mode,
            existing_dir_mode
        ),
        (0o600, 0o700, 0o600, 0o700)
    );
}

#[test]
fn token_paths_reject_escaping_reads() {
    if !isolated_home("token_paths_reject_escaping_reads") {
        return;
    }
    let home = claurst_core::config::config_home();
    std::fs::create_dir(home.join("mcp-tokens")).unwrap();
    std::fs::write(
        home.join("outside.json"),
        serde_json::to_vec(&fixture_token("outside")).unwrap(),
    )
    .unwrap();
    assert!(get_mcp_token("../outside").is_none());
    assert!(get_mcp_token(&home.join("outside").to_string_lossy()).is_none());
}

fn check_invalid_mutation(test_name: &str, operation: impl Fn(&str) -> std::io::Result<()>) {
    if !isolated_home(test_name) {
        return;
    }
    let home = claurst_core::config::config_home();
    std::fs::create_dir(home.join("mcp-tokens")).unwrap();
    let outside = home.join("outside.json");
    std::fs::write(&outside, "outside-marker").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
    let absolute = home.join("outside").to_string_lossy().into_owned();
    for name in [
        "../outside",
        absolute.as_str(),
        "nested/name",
        "nested\\name",
        "drive:name",
        "nul\0name",
        "",
    ] {
        assert_eq!(
            operation(name).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside-marker");
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}

#[test]
fn token_paths_preserve_safe_names_and_roundtrip() {
    if !isolated_home("token_paths_preserve_safe_names_and_roundtrip") {
        return;
    }
    for name in ["normal", " space name ", "équipe", ".", ".."] {
        let token = fixture_token(name);
        store_mcp_token(&token).unwrap();
        assert_eq!(get_mcp_token(name).unwrap().server_name, name);
        remove_mcp_token(name).unwrap();
        assert!(get_mcp_token(name).is_none());
        remove_mcp_token(name).unwrap();
    }
}

#[test]
fn token_paths_reject_escaping_writes() {
    check_invalid_mutation("token_paths_reject_escaping_writes", |name| {
        store_mcp_token(&fixture_token(name))
    });
}

#[test]
fn token_paths_reject_escaping_deletes() {
    check_invalid_mutation("token_paths_reject_escaping_deletes", remove_mcp_token);
}
