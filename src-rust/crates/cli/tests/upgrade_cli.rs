use std::process::Command;

fn binary_path() -> String {
    env!("CARGO_BIN_EXE_coven-code").to_string()
}

#[test]
fn upgrade_version_flag_is_routed_to_upgrade_subcommand() {
    let output = Command::new(binary_path())
        .args(["upgrade", "--version"])
        .output()
        .expect("run coven-code upgrade --version");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "upgrade --version without an argument should fail in the upgrade parser"
    );
    assert!(
        stderr.contains("--version requires an argument"),
        "stderr should report the upgrade parser error, got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !stdout.starts_with("coven-code "),
        "top-level --version should not intercept upgrade --version"
    );
}
