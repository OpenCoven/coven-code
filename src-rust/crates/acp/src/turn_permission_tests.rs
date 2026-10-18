//! Isolated real ACP dispatch/query/permission bridge tests; no provider transport.
use super::*;
use async_trait::async_trait;
use claurst_api::provider::{LlmProvider, ModelInfo};
use claurst_api::provider_error::ProviderError;
use claurst_api::provider_types::{
    ProviderCapabilities, ProviderRequest, ProviderResponse, ProviderStatus, StopReason,
    StreamEvent, SystemPromptStyle,
};
use claurst_core::types::{ContentBlock, UsageInfo};
use claurst_core::{Config, ProviderId, Settings};
use claurst_tools::{PermissionLevel, Tool, ToolResult};
use futures::Stream;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::sync::Notify;

struct FixtureProvider {
    id: ProviderId,
}

#[async_trait]
impl LlmProvider for FixtureProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }
    fn name(&self) -> &str {
        "Permission fixture"
    }
    async fn create_message(&self, _: ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        panic!("fixture must use streaming");
    }
    async fn create_message_stream(
        &self,
        request: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        let text = request.messages.last().unwrap().get_all_text();
        let names: &[&str] = match text.as_str() {
            "old" => &["first", "late"],
            "next" => &["next"],
            "capture" => &["capture"],
            _ => &[],
        };
        let mut events = vec![StreamEvent::MessageStart {
            id: "fixture".into(),
            model: "fixture".into(),
            usage: UsageInfo::default(),
        }];
        if names.is_empty() {
            events.push(StreamEvent::TextDelta {
                index: 0,
                text: "Finished fixture.".into(),
            });
        }
        for (index, name) in names.iter().enumerate() {
            events.push(StreamEvent::ContentBlockStart {
                index,
                content_block: ContentBlock::ToolUse {
                    id: format!("call-{name}"),
                    name: "PermissionFixture".into(),
                    input: json!({}),
                },
            });
            events.push(StreamEvent::InputJsonDelta {
                index,
                partial_json: json!({"name":name}).to_string(),
            });
            events.push(StreamEvent::ContentBlockStop { index });
        }
        events.extend([
            StreamEvent::MessageDelta {
                stop_reason: Some(if names.is_empty() {
                    StopReason::EndTurn
                } else {
                    StopReason::ToolUse
                }),
                usage: Some(UsageInfo::default()),
            },
            StreamEvent::MessageStop,
        ]);
        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
    async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
        Ok(ProviderStatus::Healthy)
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            thinking: false,
            image_input: false,
            pdf_input: false,
            audio_input: false,
            video_input: false,
            caching: false,
            structured_output: false,
            system_prompt_style: SystemPromptStyle::TopLevel,
        }
    }
}

struct FixtureTool {
    late_entered: Arc<Notify>,
    release_late: Arc<Notify>,
    executed: Arc<Mutex<Vec<String>>>,
    captured: Arc<Mutex<Option<ToolContext>>>,
}

#[async_trait]
impl Tool for FixtureTool {
    fn name(&self) -> &str {
        "PermissionFixture"
    }
    fn description(&self) -> &str {
        "Permission lifecycle fixture"
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object"})
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let name = input["name"].as_str().unwrap();
        if name == "capture" {
            *self.captured.lock().unwrap() = Some(ctx.clone());
            return ToolResult::success("captured context");
        }
        if name == "late" {
            self.late_entered.notify_one();
            self.release_late.notified().await;
        }
        if let Err(error) =
            ctx.check_permission_with_details(self.name(), self.description(), name, false)
        {
            return ToolResult::error(error.to_string());
        }
        self.executed.lock().unwrap().push(name.to_owned());
        ToolResult::success("approved fixture")
    }
}

async fn next_wire(output: &mut BufReader<DuplexStream>) -> Value {
    let mut line = String::new();
    let count = tokio::time::timeout(Duration::from_secs(2), output.read_line(&mut line))
        .await
        .expect("ACP output stalled")
        .unwrap();
    assert!(count > 0);
    serde_json::from_str(&line).unwrap()
}

async fn permission(output: &mut BufReader<DuplexStream>, name: &str) -> Value {
    loop {
        let message = next_wire(output).await;
        if message["method"] == "session/request_permission" {
            assert_eq!(message["params"]["toolCall"]["title"], name);
            return message;
        }
        assert!(
            message.get("id").is_none(),
            "unexpected prompt response: {message}"
        );
    }
}

async fn response(output: &mut BufReader<DuplexStream>, id: i64, reason: &str) {
    loop {
        let message = next_wire(output).await;
        assert_ne!(
            message["method"], "session/request_permission",
            "canceled late permission reached client"
        );
        if message.get("id").is_some() {
            assert_eq!(message["id"], id);
            assert_eq!(message["result"]["stopReason"], reason);
            return;
        }
    }
}

fn prompt(
    server: &Arc<crate::server::AgentServer>,
    id: i64,
    text: &str,
) -> tokio::task::JoinHandle<()> {
    server.dispatch(crate::connection::Inbound::Request {
        id: acp::RequestId::Number(id),
        method: "session/prompt".into(),
        params: Some(
            json!({"sessionId":"fixture-session", "prompt":[{"type":"text","text":text}]}),
        ),
    })
}

async fn scenario(root: &Path, max_turns: u32) {
    let config = Config {
        provider: Some("codex".into()),
        auto_commits: Some(false),
        ..Config::default()
    };
    assert!(config.hooks.is_empty());
    let mut registry = claurst_api::ProviderRegistry::new();
    registry.register(Arc::new(FixtureProvider {
        id: ProviderId::new("codex"),
    }));
    let registry = Arc::new(registry);
    let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
        api_key: "unused-fixture-key".into(),
        api_base: "http://127.0.0.1:9".into(),
        ..Default::default()
    })
    .unwrap();
    let late_entered = Arc::new(Notify::new());
    let release_late = Arc::new(Notify::new());
    let executed = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::new(Mutex::new(None));
    let settings = Settings::default();
    let runtime = AgentRuntime {
        permission_manager: Arc::new(Mutex::new(
            claurst_core::permissions::PermissionManager::new(
                config.permission_mode.clone(),
                &settings,
            ),
        )),
        settings,
        config,
        api_client: Arc::new(client),
        provider_registry: registry.clone(),
        tools: Arc::new(vec![Box::new(FixtureTool {
            late_entered: late_entered.clone(),
            release_late: release_late.clone(),
            executed: executed.clone(),
            captured: captured.clone(),
        })]),
        cost_tracker: claurst_core::CostTracker::new(),
        query_config: claurst_query::QueryConfig {
            model: "fixture".into(),
            max_turns: if max_turns >= 10 { 4 } else { max_turns },
            provider_registry: Some(registry),
            working_directory: Some(root.display().to_string()),
            ..Default::default()
        },
        mcp_manager: None,
        working_dir: root.to_path_buf(),
    };
    let (output, writer) = tokio::io::duplex(65536);
    let (mut input, reader_input) = tokio::io::duplex(65536);
    let connection = Connection::new(writer);
    let (tx, _rx) = mpsc::unbounded_channel();
    let reader = tokio::spawn(crate::connection::run_reader(
        connection.clone(),
        reader_input,
        tx,
    ));
    let server = crate::server::AgentServer::new(connection, Arc::new(runtime));
    server.sessions.insert(SessionState::new(
        acp::SessionId::new("fixture-session"),
        root.to_path_buf(),
    ));
    let mut output = BufReader::new(output);
    if max_turns >= 10 {
        let parent = prompt(&server, 1, "capture");
        response(&mut output, 1, "end_turn").await;
        parent.await.unwrap();
        let ctx = captured
            .lock()
            .unwrap()
            .take()
            .expect("query tool did not capture context");
        if max_turns == 11 {
            server
                .dispatch(crate::connection::Inbound::Notification {
                    method: "session/cancel".into(),
                    params: Some(json!({"sessionId":"fixture-session"})),
                })
                .await
                .unwrap();
            let next = prompt(&server, 2, "plain");
            response(&mut output, 2, "end_turn").await;
            next.await.unwrap();
        }
        let background = tokio::spawn(async move {
            ctx.check_permission_with_details(
                "PermissionFixture",
                "background fixture",
                "background",
                false,
            )
        });
        if max_turns == 10 {
            let request = permission(&mut output, "background").await;
            let reply = json!({"jsonrpc":"2.0", "id":request["id"], "result":{"outcome":{"outcome":"selected","optionId":"allow_once"}}});
            input
                .write_all(format!("{reply}\n").as_bytes())
                .await
                .unwrap();
            assert!(tokio::time::timeout(Duration::from_secs(2), background)
                .await
                .unwrap()
                .unwrap()
                .is_ok());
        } else {
            assert!(tokio::time::timeout(Duration::from_secs(2), background)
                .await
                .expect("canceled generation retained a background permission wait")
                .unwrap()
                .is_err());
            assert!(
                tokio::time::timeout(Duration::from_millis(100), next_wire(&mut output))
                    .await
                    .is_err(),
                "canceled background permission reached client"
            );
        }
        drop(input);
        tokio::time::timeout(Duration::from_secs(2), reader)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        return;
    }
    let old = prompt(&server, 1, "old");
    let _unanswered = permission(&mut output, "first").await;
    server
        .dispatch(crate::connection::Inbound::Notification {
            method: "session/cancel".into(),
            params: Some(json!({"sessionId":"fixture-session"})),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), late_entered.notified())
        .await
        .expect("session/cancel did not release the first permission without a client reply");
    assert!(executed.lock().unwrap().is_empty());
    if max_turns > 1 {
        // The new generation must retain its pending approval while the old turn exits.
        let next = prompt(&server, 2, "next");
        let request = permission(&mut output, "next").await;
        release_late.notify_one();
        response(&mut output, 1, "cancelled").await;
        old.await.unwrap();
        assert!(
            !next.is_finished(),
            "old turn cleanup resolved another turn's permission"
        );
        assert!(executed.lock().unwrap().is_empty());
        let reply = json!({"jsonrpc":"2.0", "id":request["id"], "result":{"outcome":{"outcome":"selected","optionId":"allow_once"}}});
        input
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .unwrap();
        response(&mut output, 2, "end_turn").await;
        next.await.unwrap();
        assert_eq!(*executed.lock().unwrap(), ["next"]);
    } else {
        release_late.notify_one();
        response(&mut output, 1, "cancelled").await;
        old.await.unwrap();
        let next = prompt(&server, 2, "plain");
        response(&mut output, 2, "end_turn").await;
        next.await.unwrap();
        assert!(executed.lock().unwrap().is_empty());
    }
    drop(input);
    tokio::time::timeout(Duration::from_secs(2), reader)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

fn run_isolated(max_turns: u32) {
    let root = std::env::temp_dir().join(format!(
        "coven-acp-permission-{}-{max_turns}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let stdout = std::fs::File::create(root.join("stdout")).unwrap();
    let stderr = std::fs::File::create(root.join("stderr")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "prompt::turn_permission_tests::isolated_worker",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("HOME", &root)
        .env("USERPROFILE", &root)
        .env("COVEN_HOME", root.join("coven"))
        .env("COVEN_CAVE_HOME", root.join("cave"))
        .env("COVEN_CODE_HOME", &root)
        .env("XDG_CONFIG_HOME", &root)
        .env("XDG_CACHE_HOME", &root)
        .env("ACP_FIXTURE_ROOT", &root)
        .env("ACP_FIXTURE_MAX_TURNS", max_turns.to_string())
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let diagnostics = std::fs::read_to_string(root.join("stderr")).unwrap();
    let marker = std::fs::read(root.join("complete")).ok();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        status.is_some_and(|status| status.success()),
        "fixture failed: {status:?}\n{diagnostics}"
    );
    assert_eq!(marker.as_deref(), Some(b"complete".as_slice()));
}

#[test]
fn session_cancel_releases_permissions_and_preserves_next_prompt() {
    run_isolated(4);
}
#[test]
fn session_cancel_wins_over_tool_round_limit() {
    run_isolated(1);
}

#[test]
fn background_context_can_request_after_parent_completion() {
    run_isolated(10);
}
#[test]
fn canceled_background_context_stays_canceled_after_next_prompt() {
    run_isolated(11);
}

#[test]
#[ignore = "owned process driver"]
fn isolated_worker() {
    let root = PathBuf::from(std::env::var_os("ACP_FIXTURE_ROOT").unwrap());
    assert_eq!(std::env::current_dir().unwrap(), root);
    assert_eq!(
        std::env::var_os("COVEN_CODE_HOME").unwrap(),
        root.as_os_str()
    );
    let max_turns = std::env::var("ACP_FIXTURE_MAX_TURNS")
        .unwrap()
        .parse()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(scenario(&root, max_turns))
    }));
    // A broken permission bridge may strand block_in_place. Never let runtime teardown hang the child.
    runtime.shutdown_timeout(Duration::from_millis(200));
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    std::fs::write(root.join("complete"), "complete").unwrap();
}
