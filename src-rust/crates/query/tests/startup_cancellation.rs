use async_trait::async_trait;
use claurst_api::{
    client::ClientConfig, AnthropicClient, LlmProvider, ModelInfo, ProviderCapabilities,
    ProviderError, ProviderRegistry, ProviderRequest, ProviderResponse, ProviderStatus,
    StreamEvent,
};
use claurst_core::{
    config::{Config, PermissionMode},
    cost::CostTracker,
    permissions::AutoPermissionHandler,
    provider_id::ProviderId,
    types::Message,
};
use claurst_query::{run_query_loop, QueryConfig, QueryEvent, QueryOutcome};
use claurst_tools::ToolContext;
use futures::Stream;
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{mpsc, Notify},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const DEADLINE: Duration = Duration::from_secs(2);
const PROVIDER: &str = "startup-fixture";

struct StartupProvider {
    id: ProviderId,
    entered: Arc<Notify>,
    dropped: Arc<AtomicBool>,
    fail: bool,
}
struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[async_trait]
impl LlmProvider for StartupProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }
    fn name(&self) -> &str {
        "Startup fixture"
    }
    async fn create_message(&self, _: ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        panic!("non-streaming request unexpected")
    }
    async fn create_message_stream(
        &self,
        _: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        let _guard = DropFlag(self.dropped.clone());
        self.entered.notify_one();
        if self.fail {
            return Err(ProviderError::Other {
                provider: self.id.clone(),
                message: "startup rejected".into(),
                status: None,
                body: None,
            });
        }
        std::future::pending().await
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
            tool_calling: false,
            thinking: false,
            image_input: false,
            pdf_input: false,
            audio_input: false,
            video_input: false,
            caching: false,
            structured_output: false,
            system_prompt_style: claurst_api::SystemPromptStyle::TopLevel,
        }
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    context: ToolContext,
    config: QueryConfig,
    client: AnthropicClient,
    messages: Vec<Message>,
}
impl Fixture {
    fn new(api_base: String, provider: Option<StartupProvider>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let cost_tracker = CostTracker::new();
        let mut config = QueryConfig::default();
        let mut tool_config = Config {
            auto_commits: Some(false),
            ..Default::default()
        };
        if let Some(provider) = provider {
            tool_config.provider = Some(PROVIDER.into());
            let mut registry = ProviderRegistry::new();
            registry.register(Arc::new(provider));
            config.provider_registry = Some(Arc::new(registry));
        }
        let context = ToolContext {
            working_dir: directory.path().to_path_buf(),
            permission_mode: PermissionMode::BypassPermissions,
            permission_handler: Arc::new(AutoPermissionHandler {
                mode: PermissionMode::BypassPermissions,
            }),
            cost_tracker,
            session_id: "startup-cancellation".into(),
            goal_turn: Default::default(),
            file_history: Default::default(),
            current_turn: Default::default(),
            non_interactive: true,
            mcp_manager: None,
            config: tool_config,
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
        };
        Self {
            _directory: directory,
            context,
            config,
            client: AnthropicClient::new(ClientConfig {
                api_key: "local-fixture".into(),
                api_base,
                request_timeout: Duration::from_secs(30),
                max_retries: 0,
                ..Default::default()
            })
            .unwrap(),
            messages: vec![Message::user("preserve this prompt")],
        }
    }
    async fn run(
        &mut self,
        cancel: CancellationToken,
        events: mpsc::UnboundedSender<QueryEvent>,
    ) -> QueryOutcome {
        run_query_loop(
            &self.client,
            &mut self.messages,
            &[],
            &self.context,
            &self.config,
            self.context.cost_tracker.clone(),
            Some(events),
            cancel,
            None,
        )
        .await
    }
    fn assert_unsampled(&self, mut events: mpsc::UnboundedReceiver<QueryEvent>) {
        assert_eq!(self.messages.len(), 1);
        assert!(matches!(
            self.messages[0].role,
            claurst_core::types::Role::User
        ));
        assert_eq!(self.messages[0].get_all_text(), "preserve this prompt");
        assert_eq!(self.context.cost_tracker.total_tokens(), 0);
        while let Ok(event) = events.try_recv() {
            assert!(
                matches!(event, QueryEvent::Status(_)),
                "unexpected sampled event: {event:?}"
            );
        }
    }
}

fn provider(fail: bool) -> (StartupProvider, Arc<Notify>, Arc<AtomicBool>) {
    let entered = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    (
        StartupProvider {
            id: ProviderId::new(PROVIDER),
            entered: entered.clone(),
            dropped: dropped.clone(),
            fail,
        },
        entered,
        dropped,
    )
}

async fn read_request(listener: &TcpListener) -> (TcpStream, Vec<u8>) {
    let (socket, _) = listener.accept().await.unwrap();
    let mut reader = BufReader::new(socket);
    let mut length = 0;
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("POST /v1/messages "));
    loop {
        line.clear();
        assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await.unwrap();
    (reader.into_inner(), body)
}

#[tokio::test]
async fn cancellation_during_pending_provider_startup_releases_future() {
    let (provider, entered, dropped) = provider(false);
    let mut fixture = Fixture::new("http://127.0.0.1:1".into(), Some(provider));
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::unbounded_channel();
    let outcome =
        {
            let query = fixture.run(cancel.clone(), tx);
            tokio::pin!(query);
            timeout(DEADLINE, async {
            tokio::select! {
                result = &mut query => panic!("startup returned before cancellation: {result:?}"),
                _ = entered.notified() => {}
            }
        }).await.expect("provider startup not reached");
            cancel.cancel();
            timeout(DEADLINE, &mut query)
                .await
                .expect("pending provider startup ignored cancellation")
        };
    assert!(matches!(outcome, QueryOutcome::Cancelled));
    assert!(dropped.load(Ordering::SeqCst));
    fixture.assert_unsampled(rx);
}

#[tokio::test]
async fn cancellation_during_pending_http_headers_closes_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut fixture = Fixture::new(format!("http://{}", listener.local_addr().unwrap()), None);
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::unbounded_channel();
    let (outcome, mut socket) = {
        let query = fixture.run(cancel.clone(), tx);
        tokio::pin!(query);
        let socket = timeout(DEADLINE, async {
            tokio::select! {
                result = &mut query => panic!("request returned before headers: {result:?}"),
                (socket, _) = read_request(&listener) => socket,
            }
        })
        .await
        .expect("HTTP request not reached");
        cancel.cancel();
        let outcome = timeout(DEADLINE, &mut query)
            .await
            .expect("pending HTTP headers ignored cancellation");
        (outcome, socket)
    };
    assert!(matches!(outcome, QueryOutcome::Cancelled));
    assert_eq!(
        timeout(DEADLINE, socket.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    fixture.assert_unsampled(rx);
}

#[tokio::test]
async fn pre_cancelled_query_never_starts_provider_or_http() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    for use_provider in [false, true] {
        let (provider, entered, dropped) = provider(false);
        let mut fixture = Fixture::new(
            format!("http://{}", listener.local_addr().unwrap()),
            use_provider.then_some(provider),
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (tx, rx) = mpsc::unbounded_channel();
        let result = timeout(DEADLINE, fixture.run(cancel, tx)).await.unwrap();
        assert!(matches!(result, QueryOutcome::Cancelled));
        assert!(!dropped.load(Ordering::SeqCst));
        assert!(timeout(Duration::from_millis(20), entered.notified())
            .await
            .is_err());
        assert!(timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err());
        fixture.assert_unsampled(rx);
    }
}

#[tokio::test]
async fn provider_startup_error_remains_an_error() {
    let (provider, _, dropped) = provider(true);
    let mut fixture = Fixture::new("http://127.0.0.1:1".into(), Some(provider));
    let (tx, rx) = mpsc::unbounded_channel();
    let result = timeout(DEADLINE, fixture.run(CancellationToken::new(), tx))
        .await
        .unwrap();
    assert!(
        matches!(result, QueryOutcome::Error(error) if error.to_string().contains("startup rejected"))
    );
    assert!(dropped.load(Ordering::SeqCst));
    fixture.assert_unsampled(rx);
}

#[tokio::test]
async fn raw_startup_auth_error_and_fallback_are_preserved() {
    for fallback in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut fixture = Fixture::new(format!("http://{}", listener.local_addr().unwrap()), None);
        fixture.config.model = "primary-model".into();
        if fallback {
            fixture.config.fallback_model = Some("fallback-model".into());
        }
        let serve = async {
            let expected = if fallback {
                vec![("primary-model", 529), ("fallback-model", 401)]
            } else {
                vec![("primary-model", 401)]
            };
            for (model, status) in expected {
                let (mut socket, body) = read_request(&listener).await;
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(body["model"], model);
                let body = r#"{"type":"error","error":{"type":"authentication_error","message":"fixture rejected"}}"#;
                socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let (result, ()) = timeout(DEADLINE, async {
            tokio::join!(fixture.run(CancellationToken::new(), tx), serve)
        })
        .await
        .unwrap();
        assert!(
            matches!(result, QueryOutcome::Error(claurst_core::error::ClaudeError::Auth(message)) if message == "fixture rejected")
        );
        fixture.assert_unsampled(rx);
    }
}
