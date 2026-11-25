use std::{fs, process::Command};

#[test]
fn both_admission_commands_identify_a_missing_policy_before_emitting_output() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("missing-policy.json");
    let cause = fs::symlink_metadata(&file).unwrap_err().to_string();
    for args in [
        &["--verify-workspace-policy-v1"][..],
        &[
            "--print",
            "--output-format",
            "stream-json",
            "--chat-workspace-policy-v1",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_coven-code"))
            .args(args)
            .arg(&file)
            .current_dir(temp.path())
            .env_clear()
            .envs(
                ["PATH", "SystemRoot"]
                    .into_iter()
                    .filter_map(|key| std::env::var_os(key).map(|value| (key, value))),
            )
            .env("HOME", temp.path())
            .env("COVEN_CODE_HOME", temp.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        for part in ["inspect workspace policy", &format!("{file:?}"), &cause] {
            assert!(stderr.contains(part), "{args:?}: {stderr}");
        }
    }
}
