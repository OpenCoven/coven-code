//! Exercise keyless session ownership through one running ACP server.
#![cfg(unix)]

use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout, Command};

struct Connection {
    input: ChildStdin,
    responses: Lines<BufReader<ChildStdout>>,
    id: u64,
}

impl Connection {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let request = json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params});
        self.input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        loop {
            let line = self
                .responses
                .next_line()
                .await
                .unwrap()
                .expect("ACP closed stdout");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == self.id {
                assert!(message.get("result").is_some(), "{message}");
                return message["result"].clone();
            }
        }
    }
}

#[tokio::test]
async fn sessions_own_working_directory_and_resume_through_stale_retry() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().canonicalize().unwrap();
    let a = root.join("workspace a");
    let b = root.join("workspace b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let fixture = root.join("claude");
    std::fs::write(&fixture, include_str!("fixtures/keyless_claude.py")).unwrap();
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_coven-code"))
        .arg("acp")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", &root)
        .env("COVEN_CODE_HOME", &root)
        .env("COVEN_CODE_CLAUDE_BIN", "./claude")
        .env("COVEN_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut rpc = Connection {
        input: child.stdin.take().unwrap(),
        responses: BufReader::new(child.stdout.take().unwrap()).lines(),
        id: 0,
    };
    tokio::time::timeout(Duration::from_secs(30), async {
        rpc.call(
            "initialize",
            json!({"protocolVersion":1,"clientCapabilities":{}}),
        )
        .await;
        let mut ids = Vec::new();
        for cwd in [&a, &b, &a] {
            let result = rpc
                .call("session/new", json!({"cwd":cwd,"mcpServers":[]}))
                .await;
            ids.push(result["sessionId"].as_str().unwrap().to_owned());
        }
        for (index, label) in [
            (0, "A1"),
            (1, "B1"),
            (0, "A2"),
            (1, "B2"),
            (2, "C1"),
            (0, "A3"),
            (2, "C2"),
            (1, "B3"),
            (0, "A4"),
        ] {
            let result = rpc
                .call(
                    "session/prompt",
                    json!({"sessionId":ids[index],"prompt":[{"type":"text","text":label}]}),
                )
                .await;
            assert_eq!(result["stopReason"], "end_turn");
        }
        drop(rpc);
        assert!(child.wait().await.unwrap().success());
    })
    .await
    .expect("ACP session fixture timed out");
    let rows: Vec<Value> = std::fs::read_to_string(root.join("launches"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        json!(rows),
        json!([
            ["A1", a, null],
            ["B1", b, null],
            ["A2", a, "fixture-A1"],
            ["B2", b, "fixture-B1"],
            ["C1", a, null],
            ["A3", a, "fixture-A1"],
            ["A3", a, null],
            ["C2", a, "fixture-C1"],
            ["B3", b, "fixture-B1"],
            ["A4", a, "fixture-A3"]
        ])
    );
}
