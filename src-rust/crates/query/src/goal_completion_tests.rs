// Scripted provider integration for logical-query goal ownership.
mod goal_completion {
    use super::*;
    use claurst_core::goal::GoalTurnContext;
    use claurst_core::{Goal, GoalStatus, GoalStore};
    use claurst_tools::goal_complete::GoalCompleteTool;

    // The real query path resolves auth and todos through config_home even
    // with a scripted provider. Isolate each case before running that path.
    fn isolated_case_completed(case: &str) -> bool {
        let test_name = format!("tests::goal_completion::{case}");
        if std::env::var("COVEN_CODE_GOAL_QUERY_TEST").as_deref() == Ok(test_name.as_str()) {
            return false;
        }
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("code");
        let coven_home = fixture.path().join("coven");
        std::fs::create_dir_all(home.join("todos")).unwrap();
        std::fs::create_dir(&coven_home).unwrap();
        std::fs::write(home.join("auth.json"), r#"{"credentials":{}}"#).unwrap();
        std::fs::write(home.join("todos/goal-query.json"), "[]").unwrap();
        let stdout_path = fixture.path().join("stdout.log");
        let stderr_path = fixture.path().join("stderr.log");
        let mut command = Command::new(std::env::current_exe().unwrap());
        for (name, _) in std::env::vars_os() {
            let text = name.to_string_lossy();
            if text.contains("API_KEY")
                || text.starts_with("COVEN")
                    && (text.contains("TEST")
                        || text.contains("FIXTURE")
                        || text.contains("GOAL_QUERY"))
            {
                command.env_remove(name);
            }
        }
        let mut child = command
            .args(["--exact", &test_name, "--nocapture", "--test-threads=1"])
            .current_dir(fixture.path())
            .env("COVEN_CODE_GOAL_QUERY_TEST", &test_name)
            .env("COVEN_CODE_GOAL_QUERY_HOME", &home)
            .env("COVEN_CODE_HOME", &home)
            .env("COVEN_HOME", &coven_home)
            .env("COVEN_CODE_GOALS", "1")
            .env("COVEN_CODE_DISABLE_MODELS_FETCH", "1")
            .env(
                "COVEN_CODE_CLAUDE_BIN",
                fixture.path().join("absent-claude"),
            )
            .env("CLAUDE_CONFIG_DIR", fixture.path().join("claude"))
            .env("COVEN_DAEMON_SOCKET", fixture.path().join("absent.sock"))
            .env_remove("COVEN_CODE_TEST_HOME")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_API_KEY")
            .stdout(std::fs::File::create(&stdout_path).unwrap())
            .stderr(std::fs::File::create(&stderr_path).unwrap())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                pending_or_error => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!(
                        "child timed out or wait failed: {pending_or_error:?}"
                    ));
                }
            }
        };
        let stdout = std::fs::read_to_string(stdout_path).unwrap();
        let stderr = std::fs::read_to_string(stderr_path).unwrap();
        assert!(
            matches!(&result, Ok(status) if status.success())
                && stdout.contains("1 passed; 0 failed")
                && stdout.contains(&format!("test {test_name} ... ok")),
            "isolated {test_name} failed ({result:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        true
    }

    type GoalEventCallback = Arc<dyn Fn(&ScriptedProviderEvent) + Send + Sync>;

    struct RecordingGoalProvider {
        inner: ScriptedProvider,
        requests: Arc<Mutex<Vec<ProviderRequest>>>,
        on_request: Option<Arc<dyn Fn(usize) + Send + Sync>>,
        on_event: Option<GoalEventCallback>,
        on_capabilities: Option<Arc<dyn Fn() + Send + Sync>>,
        pending_creation: bool,
    }

    #[async_trait]
    impl LlmProvider for RecordingGoalProvider {
        fn id(&self) -> &ProviderId {
            self.inner.id()
        }
        fn name(&self) -> &str {
            "Local goal completion fixture"
        }
        async fn create_message(
            &self,
            request: ProviderRequest,
        ) -> Result<ProviderResponse, ProviderError> {
            self.inner.create_message(request).await
        }
        async fn create_message_stream(
            &self,
            request: ProviderRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = ScriptedProviderEvent> + Send>>, ProviderError>
        {
            let count = {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request.clone());
                requests.len()
            };
            if let Some(callback) = &self.on_request {
                callback(count);
            }
            if self.pending_creation {
                return futures::future::pending().await;
            }
            let stream = self.inner.create_message_stream(request).await?;
            let callback = self.on_event.clone();
            Ok(Box::pin(stream.inspect(move |event| {
                if let Some(callback) = &callback {
                    callback(event);
                }
            })))
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            self.inner.list_models().await
        }
        async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
            self.inner.health_check().await
        }
        fn capabilities(&self) -> ProviderCapabilities {
            if let Some(callback) = &self.on_capabilities {
                callback();
            }
            self.inner.capabilities()
        }
    }

    #[derive(Default)]
    struct GoalFixture {
        provider: Option<ScriptedProvider>,
        on_request: Option<Arc<dyn Fn(usize) + Send + Sync>>,
        on_event: Option<GoalEventCallback>,
        on_capabilities: Option<Arc<dyn Fn() + Send + Sync>>,
        direct_api_base: Option<String>,
        client_retries: Option<u32>,
        fallback_model: Option<String>,
        initial_history: Option<Vec<Message>>,
        pending_creation: bool,
        cancel_token: Option<tokio_util::sync::CancellationToken>,
        hooks: std::collections::HashMap<
            claurst_core::config::HookEvent,
            Vec<claurst_core::config::HookEntry>,
        >,
    }

    struct ReplaceGoalFixture {
        path: PathBuf,
        replacement: Arc<Mutex<Option<Goal>>>,
    }

    #[async_trait]
    impl Tool for ReplaceGoalFixture {
        fn name(&self) -> &str {
            "ReplaceGoalFixture"
        }
        fn description(&self) -> &str {
            "Replace the disposable test goal"
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::None
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _input: Value, ctx: &ToolContext) -> ToolResult {
            let store = GoalStore::open(&self.path).unwrap();
            store
                .set_goal(&ctx.session_id, "replacement B must survive", Some(900))
                .unwrap();
            store.record_completed_turn(&ctx.session_id, 51, 8).unwrap();
            *self.replacement.lock().unwrap() = store.try_get_goal(&ctx.session_id).unwrap();
            ToolResult::success("Installed B")
        }
    }

    struct GoalRun {
        outcome: QueryOutcome,
        events: Vec<QueryEvent>,
        requests: Vec<ProviderRequest>,
        replacement: Option<Goal>,
        messages: Vec<Message>,
    }

    async fn run_goal_query(
        root: &Path,
        goal_turn: GoalTurnContext,
        replace: bool,
        initial_message: Option<String>,
    ) -> GoalRun {
        let completion = ContentBlock::ToolUse {
            id: "complete-a".to_string(),
            name: "GoalComplete".to_string(),
            input: serde_json::json!({"audit_summary": "finished A", "evidence": "verified A tests"}),
        };
        let mut rounds = Vec::new();
        if replace {
            rounds.push(scripted_round(
                "replace-round",
                vec![ContentBlock::ToolUse {
                    id: "replace-a".to_string(),
                    name: "ReplaceGoalFixture".to_string(),
                    input: serde_json::json!({}),
                }],
                ProviderStopReason::ToolUse,
            ));
        }
        rounds.push(scripted_round(
            "complete-round",
            vec![completion],
            ProviderStopReason::ToolUse,
        ));
        rounds.push(scripted_round(
            "final-round",
            vec![ContentBlock::Text {
                text: "Final reply after completion result.".to_string(),
            }],
            ProviderStopReason::EndTurn,
        ));
        run_goal_fixture(
            root,
            goal_turn,
            initial_message,
            GoalFixture {
                provider: Some(ScriptedProvider::new(rounds)),
                ..Default::default()
            },
        )
        .await
    }

    async fn run_goal_fixture(
        root: &Path,
        goal_turn: GoalTurnContext,
        initial_message: Option<String>,
        fixture: GoalFixture,
    ) -> GoalRun {
        // Fail before query execution if a future case omits the process guard.
        let fixture_home = PathBuf::from(
            std::env::var_os("COVEN_CODE_GOAL_QUERY_HOME").expect("isolated query fixture home"),
        );
        assert!(fixture_home.is_absolute() && fixture_home.is_dir());
        assert_eq!(claurst_core::config::config_home(), fixture_home);
        assert_eq!(
            claurst_core::AuthStore::path(),
            fixture_home.join("auth.json")
        );
        assert_eq!(
            claurst_tools::todo_write::todos_path("goal-query"),
            fixture_home.join("todos/goal-query.json")
        );
        for key in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "CODEX_API_KEY"] {
            assert!(
                std::env::var_os(key).is_none(),
                "inherited credential key: {key}"
            );
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let replacement = Arc::new(Mutex::new(None));
        let mut registry = claurst_api::ProviderRegistry::new();
        registry.register(Arc::new(RecordingGoalProvider {
            inner: fixture
                .provider
                .unwrap_or_else(|| ScriptedProvider::new(vec![final_round()])),
            requests: requests.clone(),
            on_request: fixture.on_request,
            on_event: fixture.on_event,
            on_capabilities: fixture.on_capabilities,
            pending_creation: fixture.pending_creation,
        }));
        let mut config = make_config(Some("Local test"), Some("Keep existing instruction"));
        config.model = "scripted".to_string();
        config.fallback_model = fixture.fallback_model;
        config.provider_registry = fixture
            .direct_api_base
            .is_none()
            .then(|| Arc::new(registry));
        let tracker = CostTracker::new();
        let ctx = ToolContext {
            working_dir: root.to_path_buf(),
            permission_mode: PermissionMode::BypassPermissions,
            permission_handler: Arc::new(AutoPermissionHandler {
                mode: PermissionMode::BypassPermissions,
            }),
            cost_tracker: tracker.clone(),
            session_id: "goal-query".to_string(),
            goal_turn,
            file_history: Default::default(),
            current_turn: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: None,
            config: Config {
                provider: Some(SCRIPTED_PROVIDER_ID.to_string()),
                hooks: fixture.hooks,
                ..Config::default()
            },
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
        };
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(GoalCompleteTool),
            Box::new(WriteFixtureTool),
            Box::new(ReplaceGoalFixture {
                path: root.join("goals.sqlite"),
                replacement: replacement.clone(),
            }),
        ];
        let client_config = claurst_api::client::ClientConfig {
            api_key: "unused-test-key".to_string(),
            api_base: fixture
                .direct_api_base
                .unwrap_or_else(|| "http://127.0.0.1:1".to_string()),
            max_retries: fixture.client_retries.unwrap_or(1),
            initial_retry_delay: std::time::Duration::from_millis(1),
            ..Default::default()
        };
        let client = claurst_api::AnthropicClient::new(client_config).unwrap();
        let mut messages = fixture.initial_history.unwrap_or_else(|| {
            vec![Message::user(
                initial_message.unwrap_or_else(|| "Finish A".to_string()),
            )]
        });
        let (tx, mut rx) = mpsc::unbounded_channel();
        let outcome = run_query_loop(
            &client,
            &mut messages,
            &tools,
            &ctx,
            &config,
            tracker,
            Some(tx),
            fixture.cancel_token.unwrap_or_default(),
            None,
        )
        .await;
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        let requests = std::mem::take(&mut *requests.lock().unwrap());
        let replacement = replacement.lock().unwrap().take();
        GoalRun {
            outcome,
            events,
            requests,
            replacement,
            messages,
        }
    }

    fn final_round() -> Vec<StreamEvent> {
        scripted_round(
            "final",
            vec![ContentBlock::Text {
                text: "Done.".into(),
            }],
            ProviderStopReason::EndTurn,
        )
    }

    fn tool_block(id: &str, name: &str) -> ContentBlock {
        ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input: serde_json::json!({"audit_summary":"done", "evidence":"verified"}),
        }
    }

    fn assert_completion_result(run: &GoalRun, rejected: bool) {
        assert!(
            matches!(&run.outcome, QueryOutcome::EndTurn { message, .. }
            if message.get_all_text() == "Final reply after completion result."),
            "{:?}",
            run.outcome
        );
        assert!(run.events.iter().any(|event| matches!(event,
            QueryEvent::ToolEnd { tool_id, is_error, .. }
                if tool_id == "complete-a" && *is_error == rejected)));
        let final_request = run.requests.last().unwrap();
        assert!(final_request
            .messages
            .iter()
            .any(|message| matches!(&message.content,
            MessageContent::Blocks(blocks) if blocks.iter().any(|block| matches!(block,
                ContentBlock::ToolResult { tool_use_id, is_error, .. }
                    if tool_use_id == "complete-a" && is_error.unwrap_or(false) == rejected)))));
    }

    fn assert_original_prompt(run: &GoalRun) {
        for request in &run.requests {
            let prompt = format!("{:?}", request.system_prompt);
            assert!(prompt.contains("original A objective"), "{prompt}");
            assert!(!prompt.contains("replacement B must survive"), "{prompt}");
            assert!(prompt.contains("Keep existing instruction"));
            assert_eq!(prompt.matches("original A objective").count(), 1);
        }
    }

    #[tokio::test]
    async fn pending_goal_capture_completes_same_a_and_retains_final_reply() {
        if isolated_case_completed("pending_goal_capture_completes_same_a_and_retains_final_reply")
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        let original = store
            .set_goal("goal-query", "original A objective", Some(1000))
            .unwrap();
        let run = run_goal_query(dir.path(), GoalTurnContext::Pending(path), false, None).await;
        assert_completion_result(&run, false);
        assert!(
            run.requests.last().unwrap().tools.is_empty(),
            "own completion must offer only a final response"
        );
        assert_original_prompt(&run);
        let completed = store.try_get_goal("goal-query").unwrap().unwrap();
        assert_eq!(completed.id, original.id);
        assert_eq!(completed.status, GoalStatus::Complete);
        assert_eq!(completed.tokens_used, original.tokens_used);
        assert_eq!(completed.turns_used, original.turns_used);
    }

    #[tokio::test]
    async fn later_tool_round_cannot_complete_replacement_b() {
        if isolated_case_completed("later_tool_round_cannot_complete_replacement_b") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store
            .set_goal("goal-query", "original A objective", None)
            .unwrap();
        let run = run_goal_query(dir.path(), GoalTurnContext::Pending(path), true, None).await;
        assert!(matches!(run.outcome, QueryOutcome::Error(_)));
        assert_eq!(
            run.requests.len(),
            1,
            "replacement must stop the next model request"
        );
        assert_original_prompt(&run);
        assert_eq!(store.try_get_goal("goal-query").unwrap(), run.replacement);
        assert!(!run.events.iter().any(|event| matches!(event,
            QueryEvent::ToolStart { tool_id, .. } if tool_id == "complete-a")));
    }

    #[tokio::test]
    async fn supplied_capture_and_captured_absence_are_preserved() {
        if isolated_case_completed("supplied_capture_and_captured_absence_are_preserved") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store
            .set_goal("goal-query", "original A objective", None)
            .unwrap();
        let captured = GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
        let replacement = store
            .set_goal("goal-query", "replacement B must survive", None)
            .unwrap();
        let run = run_goal_query(dir.path(), captured, false, None).await;
        assert!(matches!(run.outcome, QueryOutcome::Error(_)));
        assert!(run.requests.is_empty(), "revoked capture must not dispatch");
        assert_eq!(
            store.try_get_goal("goal-query").unwrap().unwrap(),
            replacement
        );
        let run = run_goal_query(dir.path(), GoalTurnContext::Captured(None), false, None).await;
        assert_completion_result(&run, true);
        for request in run.requests {
            assert!(!format!("{:?}", request.system_prompt).contains("replacement B must survive"));
        }
        assert_eq!(
            store.try_get_goal("goal-query").unwrap().unwrap(),
            replacement
        );
    }

    #[tokio::test]
    async fn capture_store_failure_stops_before_provider_dispatch() {
        if isolated_case_completed("capture_store_failure_stops_before_provider_dispatch") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let invalid_path = dir.path().join("invalid.sqlite");
        std::fs::write(&invalid_path, b"not a SQLite database").unwrap();
        let run = run_goal_query(
            dir.path(),
            GoalTurnContext::Pending(invalid_path.clone()),
            false,
            None,
        )
        .await;
        assert!(matches!(run.outcome, QueryOutcome::Error(_)));
        assert!(run.requests.is_empty(), "failed capture must not dispatch");
        assert_eq!(
            std::fs::read(&invalid_path).unwrap(),
            b"not a SQLite database"
        );
    }

    #[tokio::test]
    async fn synthetic_turn_uses_supplied_snapshot_and_finishes_own_reply() {
        if isolated_case_completed("synthetic_turn_uses_supplied_snapshot_and_finishes_own_reply") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = GoalStore::open(&dir.path().join("goals.sqlite")).unwrap();
        let original = store
            .set_goal("goal-query", "original A objective", Some(1000))
            .unwrap();
        let captured = GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
        let run = run_goal_query(
            dir.path(),
            captured,
            false,
            Some(claurst_core::goal_continuation_message(&original)),
        )
        .await;
        assert_completion_result(&run, false);
        assert_original_prompt(&run);
        assert_eq!(
            store.try_get_goal("goal-query").unwrap().unwrap().status,
            GoalStatus::Complete
        );
    }

    #[tokio::test]
    async fn absent_at_query_entry_cannot_acquire_a_goal_from_a_later_round() {
        if isolated_case_completed("absent_at_query_entry_cannot_acquire_a_goal_from_a_later_round")
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let run = run_goal_query(
            dir.path(),
            GoalTurnContext::Pending(path.clone()),
            true,
            None,
        )
        .await;
        assert_completion_result(&run, true);
        let store = GoalStore::open(&path).unwrap();
        assert_eq!(store.try_get_goal("goal-query").unwrap(), run.replacement);
        for request in run.requests {
            assert!(!format!("{:?}", request.system_prompt).contains("replacement B must survive"));
        }
    }

    #[tokio::test]
    async fn preparation_revocation_suppresses_dispatch() {
        if isolated_case_completed("preparation_revocation_suppresses_dispatch") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let hook_path = path.clone();
        let run = run_goal_fixture(
            dir.path(),
            GoalTurnContext::Pending(path),
            None,
            GoalFixture {
                on_capabilities: Some(Arc::new(move || {
                    GoalStore::open(&hook_path)
                        .unwrap()
                        .set_goal("goal-query", "B", Some(900))
                        .unwrap();
                })),
                ..Default::default()
            },
        )
        .await;
        assert!(
            run.requests.is_empty(),
            "capability preparation revoked A before provider dispatch"
        );
        assert!(matches!(run.outcome, QueryOutcome::Error(_)));
    }

    #[tokio::test]
    async fn response_revocation_retains_text_and_closes_unexecuted_tools() {
        if isolated_case_completed("response_revocation_retains_text_and_closes_unexecuted_tools") {
            return;
        }
        for status in [
            Some(GoalStatus::Paused),
            Some(GoalStatus::Complete),
            Some(GoalStatus::BudgetLimited),
            None,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("goals.sqlite");
            let store = GoalStore::open(&path).unwrap();
            store.set_goal("goal-query", "A", None).unwrap();
            let hook_path = path.clone();
            let run = run_goal_fixture(
                dir.path(),
                GoalTurnContext::Pending(path),
                None,
                GoalFixture {
                    provider: Some(ScriptedProvider::new(vec![
                        scripted_round(
                            "partial",
                            vec![
                                ContentBlock::Text {
                                    text: "Received before stop.".into(),
                                },
                                tool_block("must-not-write", "WriteFixture"),
                            ],
                            ProviderStopReason::ToolUse,
                        ),
                        final_round(),
                    ])),
                    on_event: Some(Arc::new(move |event| {
                        if matches!(event, Ok(StreamEvent::MessageStop)) {
                            let store = GoalStore::open(&hook_path).unwrap();
                            match &status {
                                Some(status) => {
                                    store.set_status("goal-query", status.clone()).unwrap()
                                }
                                None => store.clear_goal("goal-query").unwrap(),
                            }
                        }
                    })),
                    ..Default::default()
                },
            )
            .await;
            assert!(
                !dir.path().join("fixture.txt").exists(),
                "revocation must precede execution"
            );
            assert!(matches!(run.outcome, QueryOutcome::Error(_)));
            assert_eq!(run.requests.len(), 1);
            assert!(run
                .messages
                .iter()
                .any(|m| m.get_all_text() == "Received before stop."));
            assert!(run.messages.iter().flat_map(|m| m.content_blocks()).any(|b| matches!(b,
                ContentBlock::ToolResult { tool_use_id, is_error: Some(true), .. } if tool_use_id == "must-not-write")));
        }
    }

    #[tokio::test]
    async fn concurrent_batch_observes_replacement_before_later_tool_start() {
        if isolated_case_completed("concurrent_batch_observes_replacement_before_later_tool_start")
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let run = run_goal_fixture(
            dir.path(),
            GoalTurnContext::Pending(path),
            None,
            GoalFixture {
                provider: Some(ScriptedProvider::new(vec![
                    scripted_round(
                        "batch",
                        vec![
                            tool_block("replace", "ReplaceGoalFixture"),
                            tool_block("write", "WriteFixture"),
                        ],
                        ProviderStopReason::ToolUse,
                    ),
                    final_round(),
                ])),
                ..Default::default()
            },
        )
        .await;
        assert!(
            !dir.path().join("fixture.txt").exists(),
            "not-yet-started tool must see revocation"
        );
        assert_eq!(run.requests.len(), 1);
        assert_eq!(store.try_get_goal("goal-query").unwrap(), run.replacement);
    }

    #[tokio::test]
    async fn own_completion_summary_cannot_execute_tools() {
        if isolated_case_completed("own_completion_summary_cannot_execute_tools") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let run = run_goal_fixture(
            dir.path(),
            GoalTurnContext::Pending(path),
            None,
            GoalFixture {
                provider: Some(ScriptedProvider::new(vec![
                    scripted_round(
                        "complete",
                        vec![tool_block("complete", "GoalComplete")],
                        ProviderStopReason::ToolUse,
                    ),
                    scripted_round(
                        "summary",
                        vec![
                            ContentBlock::Text {
                                text: "Summary.".into(),
                            },
                            tool_block("write", "WriteFixture"),
                        ],
                        ProviderStopReason::ToolUse,
                    ),
                    final_round(),
                ])),
                ..Default::default()
            },
        )
        .await;
        assert!(
            !dir.path().join("fixture.txt").exists(),
            "own receipt authorizes no tools"
        );
        assert_eq!(run.requests.len(), 2);
        assert!(run.requests[1].tools.is_empty());
        assert!(run.messages.iter().any(|m| m.get_all_text() == "Summary."));
    }

    #[tokio::test]
    async fn direct_http_retry_rechecks_goal_after_429_and_529() {
        if isolated_case_completed("direct_http_retry_rechecks_goal_after_429_and_529") {
            return;
        }
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for adapter in [false, true] {
            for status in [429, 529] {
                for revoke in [false, true] {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("goals.sqlite");
                    let store = GoalStore::open(&path).unwrap();
                    store.set_goal("goal-query", "A", None).unwrap();
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let addr = listener.local_addr().unwrap();
                    let count = Arc::new(AtomicUsize::new(0));
                    let server_count = count.clone();
                    let server_path = path.clone();
                    let stop = tokio_util::sync::CancellationToken::new();
                    let server_stop = stop.clone();
                    let server = tokio::spawn(async move {
                        loop {
                            let (mut socket, _) = tokio::select! {
                                _ = server_stop.cancelled() => break,
                                result = listener.accept() => result.unwrap(),
                            };
                            let mut data = Vec::new();
                            let mut buffer = [0u8; 4096];
                            loop {
                                let n = tokio::time::timeout(
                                    std::time::Duration::from_secs(2),
                                    socket.read(&mut buffer),
                                )
                                .await
                                .unwrap()
                                .unwrap();
                                assert_ne!(n, 0);
                                data.extend_from_slice(&buffer[..n]);
                                if let Some(end) = data.windows(4).position(|s| s == b"\r\n\r\n") {
                                    let headers = std::str::from_utf8(&data[..end]).unwrap();
                                    let len = headers
                                        .lines()
                                        .find_map(|s| {
                                            s.to_ascii_lowercase()
                                                .strip_prefix("content-length:")
                                                .map(|n| n.trim().parse::<usize>().unwrap())
                                        })
                                        .unwrap();
                                    if data.len() >= end + 4 + len {
                                        break;
                                    }
                                }
                            }
                            let attempt = server_count.fetch_add(1, Ordering::SeqCst);
                            let response = if attempt == 0 {
                                if revoke {
                                    let store = GoalStore::open(&server_path).unwrap();
                                    store.set_goal("goal-query", "B", Some(900)).unwrap();
                                    store.record_completed_turn("goal-query", 21, 8).unwrap();
                                }
                                format!("HTTP/1.1 {status} Busy\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}")
                            } else {
                                let body = concat!(
                                "event: message_start\ndata: {\"message\":{\"id\":\"final\",\"model\":\"claude-test\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
                                "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"Done.\"}}\n\n",
                                "event: content_block_stop\ndata: {\"index\":0}\n\n",
                                "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
                                "event: message_stop\ndata: {}\n\n"
                            );
                                format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                            };
                            socket.write_all(response.as_bytes()).await.unwrap();
                        }
                    });
                    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                        if adapter {
                            // Invoke the actual guarded Anthropic adapter. Runtime
                            // auth deliberately prefers the local CLI when no key
                            // is stored, so a scripted registry cannot select it.
                            let provider =
                                claurst_api::providers::anthropic::AnthropicProvider::from_config(
                                    claurst_api::client::ClientConfig {
                                        api_key: "test-key".into(),
                                        api_base: format!("http://{addr}"),
                                        max_retries: 1,
                                        initial_retry_delay: std::time::Duration::from_millis(1),
                                        ..Default::default()
                                    },
                                )
                                .unwrap();
                            let turn = store.capture_active_goal("goal-query").unwrap().unwrap();
                            let request = ProviderRequest {
                                model: "claude-test".into(),
                                messages: vec![Message::user("Finish A")],
                                system_prompt: None,
                                tools: vec![],
                                max_tokens: 100,
                                temperature: None,
                                top_p: None,
                                top_k: None,
                                stop_sequences: vec![],
                                thinking: None,
                                provider_options: serde_json::json!({}),
                            };
                            match provider
                                .create_message_stream_for_goal(
                                    request,
                                    Some(&turn),
                                    GoalTurnPermission::Work,
                                )
                                .await
                            {
                                Err(_) => true,
                                Ok(mut stream) => {
                                    let mut complete = false;
                                    while let Some(event) = stream.next().await {
                                        match event {
                                            Err(_) | Ok(StreamEvent::Error { .. }) => return true,
                                            Ok(StreamEvent::MessageStop) => complete = true,
                                            _ => {}
                                        }
                                    }
                                    assert!(complete);
                                    false
                                }
                            }
                        } else {
                            let run = run_goal_fixture(
                                dir.path(),
                                GoalTurnContext::Pending(path),
                                None,
                                GoalFixture {
                                    direct_api_base: Some(format!("http://{addr}")),
                                    ..Default::default()
                                },
                            )
                            .await;
                            matches!(run.outcome, QueryOutcome::Error(_))
                        }
                    })
                    .await;
                    stop.cancel();
                    server.await.unwrap();
                    assert_eq!(
                        count.load(Ordering::SeqCst),
                        if revoke { 1 } else { 2 },
                        "status {status}, revoke {revoke}, adapter {adapter}"
                    );
                    assert_eq!(result.unwrap(), revoke);
                    let current = store.try_get_goal("goal-query").unwrap().unwrap();
                    if revoke {
                        assert_eq!(current.objective, "B");
                        assert_eq!(
                            (
                                current.tokens_used,
                                current.time_used_secs,
                                current.turns_used
                            ),
                            (21, 8, 1)
                        );
                    } else {
                        assert_eq!(current.objective, "A");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn ping_only_revocation_stops_without_waiting_for_stall_timeout() {
        if isolated_case_completed("ping_only_revocation_stops_without_waiting_for_stall_timeout") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let hook_path = path.clone();
        let run = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            run_goal_fixture(
                dir.path(),
                GoalTurnContext::Pending(path),
                None,
                GoalFixture {
                    provider: Some(ScriptedProvider::partial_then_pending(vec![
                        StreamEvent::Ping,
                    ])),
                    on_event: Some(Arc::new(move |event| {
                        if matches!(event, Ok(StreamEvent::Ping)) {
                            GoalStore::open(&hook_path)
                                .unwrap()
                                .set_status("goal-query", GoalStatus::Paused)
                                .unwrap();
                        }
                    })),
                    ..Default::default()
                },
            ),
        )
        .await;
        assert!(
            run.is_ok(),
            "goal revocation must end a Ping-only stream before the 45s stall retry"
        );
        let run = run.unwrap();
        assert!(matches!(run.outcome, QueryOutcome::Error(_)));
        assert_eq!(run.requests.len(), 1);
    }

    #[tokio::test]
    async fn own_final_response_is_not_reusable_authority() {
        if isolated_case_completed("own_final_response_is_not_reusable_authority") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let context = GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
        let run = run_goal_query(dir.path(), context.clone(), false, None).await;
        assert_completion_result(&run, false);
        let again = run_goal_fixture(dir.path(), context, None, GoalFixture::default()).await;
        assert!(
            again.requests.is_empty(),
            "successful summary consumes the completion exception"
        );
    }

    #[tokio::test]
    async fn pending_provider_creation_observes_cancel_and_goal_stop() {
        if isolated_case_completed("pending_provider_creation_observes_cancel_and_goal_stop") {
            return;
        }
        for cancel in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("goals.sqlite");
            let store = GoalStore::open(&path).unwrap();
            store.set_goal("goal-query", "A", None).unwrap();
            let token = tokio_util::sync::CancellationToken::new();
            let callback_token = token.clone();
            let callback_path = path.clone();
            let run = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                run_goal_fixture(
                    dir.path(),
                    GoalTurnContext::Pending(path),
                    None,
                    GoalFixture {
                        pending_creation: true,
                        cancel_token: Some(token),
                        on_request: Some(Arc::new(move |_| {
                            if cancel {
                                callback_token.cancel();
                            } else {
                                GoalStore::open(&callback_path)
                                    .unwrap()
                                    .set_status("goal-query", GoalStatus::Paused)
                                    .unwrap();
                            }
                        })),
                        ..Default::default()
                    },
                ),
            )
            .await
            .unwrap();
            assert_eq!(run.requests.len(), 1);
            assert_eq!(matches!(run.outcome, QueryOutcome::Cancelled), cancel);
            assert_eq!(matches!(run.outcome, QueryOutcome::Error(_)), !cancel);
            assert_eq!(
                run.messages.len(),
                1,
                "provider creation received no assistant transcript"
            );
            assert_eq!(run.messages[0].get_all_text(), "Finish A");
        }
    }

    #[tokio::test]
    async fn stalled_retry_revalidates_and_preserves_attempt_uuid() {
        if isolated_case_completed("stalled_retry_revalidates_and_preserves_attempt_uuid") {
            return;
        }
        for revoke in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("goals.sqlite");
            let store = GoalStore::open(&path).unwrap();
            store.set_goal("goal-query", "A", None).unwrap();
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let hook_path = path.clone();
            let mut partial = scripted_round(
                "partial",
                vec![ContentBlock::Text {
                    text: "Partial.".into(),
                }],
                ProviderStopReason::EndTurn,
            );
            partial.pop();
            let run = TEST_STREAM_STALL_TIMEOUT
                .scope(
                    std::time::Duration::from_millis(20),
                    run_goal_fixture(
                        dir.path(),
                        GoalTurnContext::Pending(path),
                        None,
                        GoalFixture {
                            provider: Some(ScriptedProvider::partial_then_success(
                                partial,
                                final_round(),
                            )),
                            on_capabilities: Some(Arc::new(move || {
                                if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1
                                    && revoke
                                {
                                    GoalStore::open(&hook_path)
                                        .unwrap()
                                        .set_goal("goal-query", "B", None)
                                        .unwrap();
                                }
                            })),
                            ..Default::default()
                        },
                    ),
                )
                .await;
            assert_eq!(run.requests.len(), if revoke { 1 } else { 2 });
            assert_eq!(matches!(run.outcome, QueryOutcome::Error(_)), revoke);
            let attempts: Vec<_> = run
                .messages
                .iter()
                .filter(|m| m.role == claurst_core::types::Role::Assistant)
                .collect();
            assert_eq!(attempts.len(), if revoke { 1 } else { 2 });
            assert_eq!(attempts[0].get_all_text(), "Partial.");
            assert!(attempts[0].uuid.is_some());
            if !revoke {
                assert_ne!(attempts[0].uuid, attempts[1].uuid);
            }
        }
    }

    #[tokio::test]
    async fn before_tool_hook_revocation_prevents_actual_execution() {
        if isolated_case_completed("before_tool_hook_revocation_prevents_actual_execution") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        // The actual awaited PreToolUse command changes the disposable SQLite
        // store before returning permission to execute the prepared tool.
        let mut hooks = std::collections::HashMap::new();
        hooks.insert(claurst_core::config::HookEvent::PreToolUse, vec![claurst_core::config::HookEntry {
            command: "python3 -c \"import sqlite3; c=sqlite3.connect('goals.sqlite'); c.execute(\\\"UPDATE goals SET status='paused'\\\"); c.commit()\"".into(),
            tool_filter: None, blocking: true,
        }]);
        let run = run_goal_fixture(
            dir.path(),
            GoalTurnContext::Pending(path),
            None,
            GoalFixture {
                provider: Some(ScriptedProvider::new(vec![
                    scripted_round(
                        "write",
                        vec![tool_block("write", "WriteFixture")],
                        ProviderStopReason::ToolUse,
                    ),
                    final_round(),
                ])),
                hooks,
                ..Default::default()
            },
        )
        .await;
        assert!(!dir.path().join("fixture.txt").exists());
        assert_eq!(run.requests.len(), 1);
        assert_eq!(
            store.try_get_goal("goal-query").unwrap().unwrap().status,
            GoalStatus::Paused
        );
        assert!(run.events.iter().any(|e| matches!(e, QueryEvent::ToolEnd { tool_id, is_error: true, .. } if tool_id == "write")));
    }

    fn sse_response(text: &str, tokens: u64, complete: bool) -> String {
        let mut body = format!(
            "event: message_start\ndata: {{\"message\":{{\"id\":\"fixture\",\"model\":\"claude-test\",\"usage\":{{\"input_tokens\":{tokens},\"output_tokens\":0}}}}}}\n\nevent: content_block_start\ndata: {{\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":{}}}}}\n\nevent: content_block_stop\ndata: {{\"index\":0}}\n\n",
            serde_json::to_string(text).unwrap()
        );
        if complete {
            body.push_str("event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {}\n\n");
        }
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    async fn serve_goal_responses(
        response: impl Fn(usize) -> String + Send + 'static,
    ) -> (
        String,
        Arc<std::sync::atomic::AtomicUsize>,
        tokio_util::sync::CancellationToken,
        tokio::task::JoinHandle<()>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server_count = count.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let server_stop = stop.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = tokio::select! {
                    _ = server_stop.cancelled() => break,
                    result = listener.accept() => result.unwrap(),
                };
                let mut data = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let n = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        socket.read(&mut buffer),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buffer[..n]);
                    if let Some(end) = data.windows(4).position(|s| s == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&data[..end]).unwrap();
                        let len = headers
                            .lines()
                            .find_map(|s| {
                                s.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|n| n.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        if data.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                if data.is_empty() {
                    continue;
                }
                let attempt = server_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // The callback runs after the entire actual request was received.
                let body = response(attempt);
                let _ = socket.write_all(body.as_bytes()).await;
            }
        });
        (format!("http://{addr}"), count, stop, server)
    }

    #[tokio::test]
    async fn fallback_reentry_rechecks_original_goal() {
        if isolated_case_completed("fallback_reentry_rechecks_original_goal") {
            return;
        }
        for revoke in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("goals.sqlite");
            let store = GoalStore::open(&path).unwrap();
            store.set_goal("goal-query", "A", None).unwrap();
            let hook_path = path.clone();
            let (base, count, stop, server) = serve_goal_responses(move |attempt| {
                if attempt == 0 {
                    if revoke {
                        GoalStore::open(&hook_path)
                            .unwrap()
                            .set_goal("goal-query", "B", None)
                            .unwrap();
                    }
                    "HTTP/1.1 529 Busy\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into()
                } else {
                    sse_response("Fallback complete.", 1, true)
                }
            })
            .await;
            let run = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                run_goal_fixture(
                    dir.path(),
                    GoalTurnContext::Pending(path),
                    None,
                    GoalFixture {
                        direct_api_base: Some(base),
                        client_retries: Some(0),
                        fallback_model: Some("claude-test-fallback".into()),
                        ..Default::default()
                    },
                ),
            )
            .await;
            stop.cancel();
            server.await.unwrap();
            let run = run.unwrap();
            assert_eq!(
                count.load(std::sync::atomic::Ordering::SeqCst),
                if revoke { 1 } else { 2 }
            );
            assert_eq!(matches!(run.outcome, QueryOutcome::Error(_)), revoke);
        }
    }

    #[tokio::test]
    async fn cancellation_during_direct_http_backoff_is_bounded() {
        if isolated_case_completed("cancellation_during_direct_http_backoff_is_bounded") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        GoalStore::open(&path)
            .unwrap()
            .set_goal("goal-query", "A", None)
            .unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let (sent, received) = tokio::sync::oneshot::channel();
        let sent = Mutex::new(Some(sent));
        let (base, count, stop, server) = serve_goal_responses(move |_| {
            if let Some(sent) = sent.lock().unwrap().take() { let _ = sent.send(()); }
            "HTTP/1.1 429 Busy\r\nRetry-After: 10\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into()
        }).await;
        let query = run_goal_fixture(
            dir.path(),
            GoalTurnContext::Pending(path),
            None,
            GoalFixture {
                direct_api_base: Some(base),
                cancel_token: Some(cancel.clone()),
                ..Default::default()
            },
        );
        tokio::pin!(query);
        let run = tokio::select! {
            outcome = &mut query => panic!("query finished before cancellation: {:?}", outcome.outcome),
            _ = received => {
                // The only available response is a 10-second retry delay.
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                cancel.cancel();
                tokio::time::timeout(std::time::Duration::from_secs(1), &mut query).await
            }
        };
        stop.cancel();
        server.await.unwrap();
        let run = run.unwrap();
        assert!(matches!(run.outcome, QueryOutcome::Cancelled));
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(run.messages.len(), 1);
    }

    #[tokio::test]
    async fn own_completion_replacement_before_final_dispatch_is_rejected() {
        if isolated_case_completed("own_completion_replacement_before_final_dispatch_is_rejected") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let hook_path = path.clone();
        let replacement = Arc::new(Mutex::new(None));
        let expected = replacement.clone();
        let run = run_goal_fixture(
            dir.path(),
            GoalTurnContext::Pending(path),
            None,
            GoalFixture {
                provider: Some(ScriptedProvider::new(vec![
                    scripted_round(
                        "complete",
                        vec![tool_block("complete", "GoalComplete")],
                        ProviderStopReason::ToolUse,
                    ),
                    final_round(),
                ])),
                on_capabilities: Some(Arc::new(move || {
                    if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                        let store = GoalStore::open(&hook_path).unwrap();
                        assert_eq!(
                            store.try_get_goal("goal-query").unwrap().unwrap().status,
                            GoalStatus::Complete
                        );
                        *expected.lock().unwrap() =
                            Some(store.set_goal("goal-query", "B", Some(900)).unwrap());
                    }
                })),
                ..Default::default()
            },
        )
        .await;
        assert!(matches!(run.outcome, QueryOutcome::Error(_)));
        assert_eq!(run.requests.len(), 1);
        assert_eq!(
            store.try_get_goal("goal-query").unwrap(),
            *replacement.lock().unwrap()
        );
        assert!(run.events.iter().any(|e| matches!(e, QueryEvent::ToolEnd { tool_id, is_error: false, .. } if tool_id == "complete")));
    }

    #[tokio::test]
    async fn raw_ping_and_premature_eof_do_not_consume_final_response_receipt() {
        if isolated_case_completed(
            "raw_ping_and_premature_eof_do_not_consume_final_response_receipt",
        ) {
            return;
        }
        for ping_only in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("goals.sqlite");
            let store = GoalStore::open(&path).unwrap();
            store.set_goal("goal-query", "A", None).unwrap();
            let context =
                GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
            store
                .complete_goal_turn(context.captured().unwrap())
                .unwrap();
            let (base, count, stop, server) = serve_goal_responses(move |_| {
                if ping_only {
                    let body = "event: ping\ndata: {}\n\n";
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                } else { sse_response("Partial summary.", 1, false) }
            }).await;
            let run = run_goal_fixture(
                dir.path(),
                context.clone(),
                None,
                GoalFixture {
                    direct_api_base: Some(base),
                    ..Default::default()
                },
            )
            .await;
            stop.cancel();
            server.await.unwrap();
            assert!(matches!(run.outcome, QueryOutcome::Error(_)));
            assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
            if !ping_only {
                assert!(run
                    .messages
                    .iter()
                    .any(|m| m.get_all_text() == "Partial summary."));
            }
            let retry = run_goal_fixture(dir.path(), context, None, GoalFixture::default()).await;
            assert!(matches!(retry.outcome, QueryOutcome::EndTurn { .. }));
            assert_eq!(retry.requests.len(), 1);
            assert!(retry.requests[0].tools.is_empty());
        }
    }

    #[tokio::test]
    async fn nested_compaction_revocation_restores_original_history() {
        if isolated_case_completed("nested_compaction_revocation_restores_original_history") {
            return;
        }
        for mode in ["proactive", "reactive", "collapse"] {
            check_nested_compaction_handoff(mode, NestedCompactionChange::Pause).await;
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum NestedCompactionChange {
        Active,
        Pause,
        Replace,
        Failure,
        Cancel,
        OwnCompletion,
    }

    async fn check_nested_compaction_handoff(mode: &'static str, change: NestedCompactionChange) {
        use serde_json::json;
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_util::sync::CancellationToken;

        std::env::set_var(
            "COVEN_CODE_FEATURE_REACTIVE_COMPACT",
            if mode == "proactive" { "0" } else { "1" },
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        let original_goal = store.set_goal("goal-query", "A", Some(999_999)).unwrap();
        let turn = GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
        let child = r1_context(
            dir.path(),
            "goal-query",
            turn.for_child_work(),
            Config::default(),
        );
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let recorded = requests.clone();
        let replacement = Arc::new(Mutex::new(None));
        let installed = replacement.clone();
        let cancel = CancellationToken::new();
        let server_cancel = cancel.clone();
        let stop = CancellationToken::new();
        let server_stop = stop.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = tokio::select! {
                    _ = server_stop.cancelled() => break,
                    result = listener.accept() => result.unwrap(),
                };
                let request = tokio::time::timeout(Duration::from_secs(2), async {
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        header.push(socket.read_u8().await.unwrap());
                    }
                    let header = String::from_utf8(header).unwrap();
                    let len = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|len| len.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    let mut raw = vec![0; len];
                    socket.read_exact(&mut raw).await.unwrap();
                    serde_json::from_slice::<Value>(&raw).unwrap()
                })
                .await
                .unwrap();
                let attempt = {
                    let mut requests = recorded.lock().unwrap();
                    requests.push(request);
                    requests.len()
                };
                let response = match attempt {
                    1 => {
                        let tokens = if mode == "reactive" { 92_000 } else { 198_000 };
                        let mut response =
                            sse_response("Received before nested compaction.", tokens, true);
                        // Exercise closure of work already sampled but not executed.
                        // The active control uses a normal end_turn response.
                        if change != NestedCompactionChange::Active {
                            let (_, body) = response.split_once("\r\n\r\n").unwrap();
                            let body = body.replace("event: message_delta", concat!(
                                "event: content_block_start\ndata: {\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"queued-write\",\"name\":\"WriteFixture\",\"input\":{}}}\n\n",
                                "event: content_block_stop\ndata: {\"index\":1}\n\nevent: message_delta"
                            )).replace("\"stop_reason\":\"end_turn\"", "\"stop_reason\":\"tool_use\"");
                            response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        }
                        response
                    }
                    2 => {
                        match change {
                            NestedCompactionChange::OwnCompletion => {
                                let result = GoalCompleteTool.execute(
                                    json!({"audit_summary":"child audit", "evidence":"local compaction barrier"}),
                                    &child,
                                ).await;
                                assert!(!result.is_error, "{}", result.content);
                                assert_eq!(
                                    child.goal_turn.validate("goal-query"),
                                    Ok(GoalTurnPermission::ReturnToForeground)
                                );
                            }
                            NestedCompactionChange::Pause => {
                                GoalStore::open(&path)
                                    .unwrap()
                                    .set_status("goal-query", GoalStatus::Paused)
                                    .unwrap();
                            }
                            NestedCompactionChange::Replace => {
                                let store = GoalStore::open(&path).unwrap();
                                store.set_goal("goal-query", "B", Some(700)).unwrap();
                                store.record_completed_turn("goal-query", 31, 7).unwrap();
                                *installed.lock().unwrap() =
                                    store.try_get_goal("goal-query").unwrap();
                            }
                            NestedCompactionChange::Cancel => {
                                server_cancel.cancel();
                                // Keep the request pending so the cancellation branch wins.
                                server_stop.cancelled().await;
                                break;
                            }
                            NestedCompactionChange::Active | NestedCompactionChange::Failure => {}
                        }
                        if change == NestedCompactionChange::Failure {
                            "HTTP/1.1 400 Bad Request\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into()
                        } else if change == NestedCompactionChange::Pause {
                            "HTTP/1.1 429 Busy\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
                                .into()
                        } else {
                            sse_response("Completed compact summary.", 1, true)
                        }
                    }
                    _ => sse_response("Foreground final summary.", 1, true),
                };
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    socket.write_all(response.as_bytes()),
                )
                .await
                .unwrap();
            }
        });
        let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
            api_key: "local-fixture".into(),
            api_base: base,
            max_retries: 1,
            initial_retry_delay: Duration::from_millis(1),
            ..Default::default()
        })
        .unwrap();
        let ctx = r1_context(dir.path(), "goal-query", turn.clone(), Config::default());
        let config = QueryConfig {
            model: "claude-test".into(),
            system_prompt: Some("Local nested compaction fixture".into()),
            max_turns: 5,
            ..Default::default()
        };
        let original: Vec<_> = (0..12)
            .map(|i| Message::user(format!("Exact history {i}")))
            .collect();
        let mut messages = original.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_query_loop(
                &client,
                &mut messages,
                &[Box::new(WriteFixtureTool)],
                &ctx,
                &config,
                ctx.cost_tracker.clone(),
                Some(tx),
                cancel,
                None,
            ),
        )
        .await;
        stop.cancel();
        server.await.unwrap();
        let outcome = result.expect("bounded nested compaction query");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let requests = requests.lock().unwrap();
        let own = change == NestedCompactionChange::OwnCompletion;
        assert_eq!(
            requests.len(),
            if own { 3 } else { 2 },
            "{mode} {change:?}: {outcome:?}"
        );
        let compact_request = requests[1].to_string();
        assert!(
            compact_request.contains(if mode == "collapse" {
                "EMERGENCY CONTEXT COLLAPSE"
            } else {
                "conversation_to_summarize"
            }),
            "{mode} {change:?}: {compact_request}"
        );
        assert!(!dir.path().join("fixture.txt").exists());
        assert!(!events
            .iter()
            .any(|event| matches!(event, QueryEvent::ToolStart { .. })));
        let successful = own || change == NestedCompactionChange::Active;
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, QueryEvent::TurnComplete { .. }))
                .count(),
            usize::from(successful)
        );
        if successful {
            assert!(
                matches!(outcome, QueryOutcome::EndTurn { .. }),
                "{mode} {change:?}: {outcome:?}"
            );
        } else if change == NestedCompactionChange::Cancel {
            assert!(
                matches!(outcome, QueryOutcome::Cancelled),
                "{mode}: {outcome:?}"
            );
        } else {
            assert!(
                matches!(outcome, QueryOutcome::Error(_)),
                "{mode} {change:?}: {outcome:?}"
            );
        }
        let sampled: Vec<_> = messages
            .iter()
            .filter(|m| m.get_all_text() == "Received before nested compaction.")
            .collect();
        assert_eq!(sampled.len(), 1);
        let uuid = sampled[0].uuid.as_deref().unwrap();
        uuid::Uuid::parse_str(uuid).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event,
                    QueryEvent::DurableMessage { message } if message.uuid.as_deref() == Some(uuid)
                ))
                .count(),
            1
        );
        if change != NestedCompactionChange::Active {
            assert_eq!(
                serde_json::to_value(&messages[..12]).unwrap(),
                serde_json::to_value(&original).unwrap()
            );
            assert_eq!(messages.len(), if own { 15 } else { 14 });
            assert_eq!(events.iter().filter(|event| matches!(event,
                QueryEvent::DurableMessage { message } if message.get_tool_result_blocks().iter().any(|block| matches!(block,
                    ContentBlock::ToolResult { tool_use_id, is_error: Some(true), .. } if tool_use_id == "queued-write"
                ))
            )).count(), 1);
            assert_eq!(messages.iter().flat_map(Message::get_tool_result_blocks).filter(|block| matches!(block,
                ContentBlock::ToolResult { tool_use_id, is_error: Some(true), .. } if tool_use_id == "queued-write"
            )).count(), 1);
        } else {
            assert!(messages[0]
                .get_all_text()
                .contains("Completed compact summary."));
        }
        let current = store.try_get_goal("goal-query").unwrap().unwrap();
        if own {
            assert_eq!(current.id, original_goal.id);
            assert_eq!(current.status, GoalStatus::Complete);
            assert!(
                turn.validate("goal-query").is_err(),
                "coherent final consumes receipt"
            );
            assert!(requests[2]
                .get("tools")
                .is_none_or(|tools| tools.as_array().unwrap().is_empty()));
            assert!(requests[2]
                .to_string()
                .contains("Received before nested compaction."));
            assert_eq!(
                messages.last().unwrap().get_all_text(),
                "Foreground final summary."
            );
            assert_ne!(messages.last().unwrap().uuid.as_deref(), Some(uuid));
        } else if change == NestedCompactionChange::Replace {
            assert_eq!(Some(current), *replacement.lock().unwrap());
            assert!(turn.validate("goal-query").is_err());
        } else if change == NestedCompactionChange::Pause {
            assert_eq!(current.status, GoalStatus::Paused);
            assert!(turn.validate("goal-query").is_err());
        } else {
            assert_eq!(current, original_goal);
            assert_eq!(turn.validate("goal-query"), Ok(GoalTurnPermission::Work));
        }
    }

    #[tokio::test]
    async fn goal_r2_proactive_compaction_hands_completion_to_foreground() {
        if isolated_case_completed("goal_r2_proactive_compaction_hands_completion_to_foreground") {
            return;
        }
        for change in [
            NestedCompactionChange::Active,
            NestedCompactionChange::Pause,
            NestedCompactionChange::Replace,
            NestedCompactionChange::Failure,
            NestedCompactionChange::Cancel,
            NestedCompactionChange::OwnCompletion,
        ] {
            check_nested_compaction_handoff("proactive", change).await;
        }
    }

    #[tokio::test]
    async fn goal_r2_reactive_compaction_hands_completion_to_foreground() {
        if isolated_case_completed("goal_r2_reactive_compaction_hands_completion_to_foreground") {
            return;
        }
        for change in [
            NestedCompactionChange::Active,
            NestedCompactionChange::Pause,
            NestedCompactionChange::Replace,
            NestedCompactionChange::Failure,
            NestedCompactionChange::Cancel,
            NestedCompactionChange::OwnCompletion,
        ] {
            check_nested_compaction_handoff("reactive", change).await;
        }
    }

    #[tokio::test]
    async fn goal_r2_collapse_hands_completion_to_foreground() {
        if isolated_case_completed("goal_r2_collapse_hands_completion_to_foreground") {
            return;
        }
        for change in [
            NestedCompactionChange::Active,
            NestedCompactionChange::Pause,
            NestedCompactionChange::Replace,
            NestedCompactionChange::Failure,
            NestedCompactionChange::Cancel,
            NestedCompactionChange::OwnCompletion,
        ] {
            check_nested_compaction_handoff("collapse", change).await;
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum BudgetCompletion {
        Active,
        ToolThenFinal,
        DuringResponse,
    }

    async fn check_goal_usd_budget(
        direct: bool,
        completion: BudgetCompletion,
        stop: &'static str,
        limit: f64,
        cancel_response: bool,
    ) {
        use serde_json::json;
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().unwrap();
        let store = GoalStore::open(&dir.path().join("goals.sqlite")).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let turn = GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
        let child = Arc::new(r1_context(
            dir.path(),
            "goal-query",
            turn.for_child_work(),
            Config::default(),
        ));
        let complete_child = Arc::new(move || {
            let result = GoalCompleteTool.execute(
                json!({"audit_summary":"budget handoff audit", "evidence":"local usage barrier"}),
                &child,
            ).now_or_never().expect("GoalComplete performs synchronous store work");
            assert!(!result.is_error, "{}", result.content);
        });
        let cancel = tokio_util::sync::CancellationToken::new();
        let collector_cancel = cancel.clone();
        let collector_complete = complete_child.clone();
        let completion_block = tool_block("complete-budget", "GoalComplete");
        let response_blocks = if stop == "tool_use" {
            vec![
                ContentBlock::Text {
                    text: "Final budget content retained.".into(),
                },
                tool_block("budget-write", "WriteFixture"),
            ]
        } else {
            vec![ContentBlock::Text {
                text: "Final budget content retained.".into(),
            }]
        };
        let stop_reason = match stop {
            "end_turn" => ProviderStopReason::EndTurn,
            "stop_sequence" => ProviderStopReason::StopSequence,
            "max_tokens" => ProviderStopReason::MaxTokens,
            "tool_use" => ProviderStopReason::ToolUse,
            _ => unreachable!(),
        };
        let mut response_round =
            scripted_round("budget-wire", response_blocks.clone(), stop_reason);
        for event in &mut response_round {
            match event {
                StreamEvent::MessageStart { usage, .. } => usage.input_tokens = 1000,
                StreamEvent::MessageDelta {
                    usage: Some(usage), ..
                } => usage.output_tokens = 1000,
                _ => {}
            }
        }
        let mut rounds = Vec::new();
        if completion == BudgetCompletion::ToolThenFinal {
            rounds.push(scripted_round(
                "budget-wire",
                vec![completion_block.clone()],
                ProviderStopReason::ToolUse,
            ));
        }
        rounds.push(response_round.clone());
        if completion == BudgetCompletion::DuringResponse {
            rounds.push(response_round);
        }
        let requests = Arc::new(Mutex::new(Vec::<ProviderRequest>::new()));
        let http_requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let recorded = http_requests.clone();
        let provider_complete = complete_child.clone();
        let completed = AtomicBool::new(false);
        let mut registry = claurst_api::ProviderRegistry::new();
        let provider = if cancel_response {
            // Keep the response pending after usage, with neither MessageStop nor EOF.
            let mut partial = rounds.remove(0);
            assert!(matches!(partial.pop(), Some(StreamEvent::MessageStop)));
            ScriptedProvider::partial_then_pending(partial)
        } else {
            ScriptedProvider::new(rounds)
        };
        registry.register(Arc::new(RecordingGoalProvider {
            inner: provider,
            requests: requests.clone(),
            on_request: None,
            on_capabilities: None,
            pending_creation: false,
            on_event: Some(Arc::new(move |event| {
                if completion == BudgetCompletion::DuringResponse
                    && matches!(event, Ok(StreamEvent::MessageStop))
                    && !completed.swap(true, Ordering::SeqCst)
                {
                    provider_complete();
                }
            })),
        }));
        let stopped = Arc::new(AtomicBool::new(false));
        let server_stop = stopped.clone();
        let (usage_seen_tx, usage_seen_rx) = std::sync::mpsc::channel();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            while !server_stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                let (mut socket, _) = match listener.accept() {
                    Ok(socket) => socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut header = Vec::new();
                let mut byte = [0];
                while !header.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                    assert!(header.len() < 64_000);
                }
                let header = String::from_utf8(header).unwrap();
                let len = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|len| len.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                let mut raw = vec![0; len];
                socket.read_exact(&mut raw).unwrap();
                let attempt = {
                    let mut requests = recorded.lock().unwrap();
                    requests.push(serde_json::from_slice(&raw).unwrap());
                    requests.len()
                };
                assert!(
                    attempt <= 2,
                    "no additional work after final or budget outcome"
                );
                let tool_round = completion == BudgetCompletion::ToolThenFinal && attempt == 1;
                let mut body = if tool_round {
                    r1_sse(serde_json::to_value(&completion_block).unwrap(), "tool_use")
                } else {
                    let mut body = r1_sse(serde_json::to_value(&response_blocks[0]).unwrap(), stop)
                        .replace("\"input_tokens\":1,", "\"input_tokens\":1000,")
                        .replace("\"output_tokens\":1}", "\"output_tokens\":1000}");
                    if stop == "tool_use" {
                        let tool = serde_json::to_string(&response_blocks[1]).unwrap();
                        body = body.replace("event: message_delta", &format!("event: content_block_start\ndata: {{\"index\":1,\"content_block\":{tool}}}\n\nevent: content_block_stop\ndata: {{\"index\":1}}\n\nevent: message_delta"));
                    }
                    body
                };
                let header = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                if cancel_response {
                    body.truncate(body.find("event: message_stop").unwrap());
                    socket.write_all(header.as_bytes()).unwrap();
                    socket.write_all(body.as_bytes()).unwrap();
                    // No MessageStop or EOF until the cancelled query has returned.
                    while !server_stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                } else if completion == BudgetCompletion::DuringResponse && attempt == 1 {
                    let terminal = body.split_off(body.find("event: message_stop").unwrap());
                    socket.write_all(header.as_bytes()).unwrap();
                    socket.write_all(body.as_bytes()).unwrap();
                    // Complete only after the real query has received billable usage.
                    usage_seen_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    complete_child();
                    let _ = socket.write_all(terminal.as_bytes());
                } else {
                    socket.write_all(header.as_bytes()).unwrap();
                    let _ = socket.write_all(body.as_bytes());
                }
            }
        });
        let mut ctx = r1_context(dir.path(), "goal-query", turn.clone(), Config::default());
        let mut config = QueryConfig {
            model: "claude-test".into(),
            max_turns: 5,
            max_budget_usd: Some(limit),
            system_prompt: Some("Local USD budget fixture".into()),
            ..Default::default()
        };
        if !direct {
            ctx.config.provider = Some(SCRIPTED_PROVIDER_ID.into());
            config.model = "scripted".into();
            config.provider_registry = Some(Arc::new(registry));
        }
        let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
            api_key: "local-budget-fixture".into(),
            api_base: base,
            max_retries: 0,
            ..Default::default()
        })
        .unwrap();
        let mut messages = vec![Message::user("Finish A within the cost cap")];
        let (tx, mut rx) = mpsc::unbounded_channel();
        let collector = tokio::spawn(async move {
            let mut events = Vec::new();
            let mut cancelled_at_usage = false;
            while let Some(event) = rx.recv().await {
                if matches!(
                    &event,
                    QueryEvent::Stream(AnthropicStreamEvent::MessageDelta { .. })
                ) {
                    let _ = usage_seen_tx.send(());
                    if cancel_response && !cancelled_at_usage {
                        cancelled_at_usage = true;
                        if completion == BudgetCompletion::DuringResponse {
                            collector_complete();
                        }
                        collector_cancel.cancel();
                    }
                }
                events.push(event);
            }
            events
        });
        let result = tokio::time::timeout(
            Duration::from_secs(6),
            run_query_loop(
                &client,
                &mut messages,
                &[Box::new(GoalCompleteTool), Box::new(WriteFixtureTool)],
                &ctx,
                &config,
                ctx.cost_tracker.clone(),
                Some(tx),
                cancel.clone(),
                None,
            ),
        )
        .await;
        stopped.store(true, Ordering::SeqCst);
        server.join().unwrap();
        let events = collector.await.unwrap();
        let outcome = result.expect("bounded goal-budget query");
        let over = limit
            <= if completion == BudgetCompletion::ToolThenFinal {
                0.09009
            } else {
                0.09
            };
        let calls = if cancel_response {
            1
        } else if completion == BudgetCompletion::ToolThenFinal
            || completion == BudgetCompletion::DuringResponse && !over
        {
            2
        } else {
            1
        };
        let tokens = match completion {
            BudgetCompletion::ToolThenFinal => 1001,
            BudgetCompletion::DuringResponse if !over && !cancel_response => 2000,
            _ => 1000,
        };
        assert_eq!(ctx.cost_tracker.input_tokens(), tokens);
        assert_eq!(ctx.cost_tracker.output_tokens(), tokens);
        let expected_cost = tokens as f64 * 0.00009;
        assert!((ctx.cost_tracker.total_cost_usd() - expected_cost).abs() < 1e-9);
        assert_eq!(
            if direct {
                http_requests.lock().unwrap().len()
            } else {
                requests.lock().unwrap().len()
            },
            calls
        );
        if cancel_response {
            assert!(cancel.is_cancelled());
            assert!(!events.iter().any(|event| matches!(
                event,
                QueryEvent::Stream(AnthropicStreamEvent::MessageStop)
            )));
            assert!(matches!(outcome, QueryOutcome::Cancelled), "{outcome:?}");
        } else if over {
            assert!(
                matches!(outcome, QueryOutcome::BudgetExceeded { cost_usd, limit_usd } if (cost_usd-expected_cost).abs()<1e-9 && limit_usd==limit),
                "{outcome:?}"
            );
        } else {
            match stop {
                "max_tokens" => assert!(
                    matches!(outcome, QueryOutcome::MaxTokens { .. }),
                    "{outcome:?}"
                ),
                "tool_use" => assert!(matches!(outcome, QueryOutcome::Error(_)), "{outcome:?}"),
                _ => assert!(
                    matches!(outcome, QueryOutcome::EndTurn { .. }),
                    "{outcome:?}"
                ),
            }
        }
        let succeeds = !cancel_response && !over && matches!(stop, "end_turn" | "stop_sequence");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, QueryEvent::TurnComplete { .. }))
                .count(),
            usize::from(succeeds)
        );
        let received: Vec<_> = messages
            .iter()
            .filter(|m| m.get_all_text() == "Final budget content retained.")
            .collect();
        assert_eq!(
            received.len(),
            if completion == BudgetCompletion::DuringResponse && !over && !cancel_response {
                2
            } else {
                1
            }
        );
        let mut ids = std::collections::HashSet::new();
        for message in received {
            let id = message.uuid.as_deref().unwrap();
            uuid::Uuid::parse_str(id).unwrap();
            assert!(ids.insert(id));
            assert_eq!(events.iter().filter(|e| matches!(e, QueryEvent::DurableMessage { message } if message.uuid.as_deref()==Some(id))).count(), 1);
        }
        assert!(!dir.path().join("fixture.txt").exists());
        assert!(!events.iter().any(
            |e| matches!(e, QueryEvent::ToolStart { tool_name, .. } if tool_name=="WriteFixture")
        ));
        if stop == "tool_use" {
            assert_eq!(events.iter().filter(|event| matches!(event,
                QueryEvent::DurableMessage { message } if message.get_tool_result_blocks().iter().any(|block| matches!(block,
                    ContentBlock::ToolResult { tool_use_id, is_error: Some(true), .. } if tool_use_id == "budget-write"
                ))
            )).count(), 1);
            assert_eq!(messages.iter().flat_map(Message::get_tool_result_blocks).filter(|b| matches!(b, ContentBlock::ToolResult { tool_use_id, is_error: Some(true), .. } if tool_use_id=="budget-write")).count(), 1);
        }
        if completion == BudgetCompletion::Active {
            assert_eq!(turn.validate("goal-query"), Ok(GoalTurnPermission::Work));
        } else {
            assert_eq!(
                store.try_get_goal("goal-query").unwrap().unwrap().status,
                GoalStatus::Complete
            );
            if completion == BudgetCompletion::DuringResponse && (over || cancel_response) {
                assert_eq!(
                    turn.validate("goal-query"),
                    Ok(GoalTurnPermission::FinalResponse),
                    "no final request consumed receipt"
                );
            } else {
                assert!(
                    turn.validate("goal-query").is_err(),
                    "coherent final consumed receipt"
                );
                if direct {
                    assert!(http_requests.lock().unwrap()[1]["tools"]
                        .as_array()
                        .unwrap()
                        .is_empty());
                } else {
                    assert!(requests.lock().unwrap()[1].tools.is_empty());
                }
            }
        }
    }

    async fn check_final_usd_budget_matrix(direct: bool) {
        for limit in [1.0, 0.05] {
            check_goal_usd_budget(direct, BudgetCompletion::Active, "end_turn", limit, false).await;
        }
        for limit in [1.0, 0.05, 0.09009] {
            for stop in ["end_turn", "stop_sequence", "max_tokens", "tool_use"] {
                check_goal_usd_budget(direct, BudgetCompletion::ToolThenFinal, stop, limit, false)
                    .await;
            }
        }
    }

    #[tokio::test]
    async fn goal_r3_provider_final_preserves_usd_budget() {
        if isolated_case_completed("goal_r3_provider_final_preserves_usd_budget") {
            return;
        }
        check_final_usd_budget_matrix(false).await;
    }

    #[tokio::test]
    async fn goal_r3_direct_final_preserves_usd_budget() {
        if isolated_case_completed("goal_r3_direct_final_preserves_usd_budget") {
            return;
        }
        check_final_usd_budget_matrix(true).await;
    }

    #[tokio::test]
    async fn goal_r3_provider_handoff_preserves_usd_budget() {
        if isolated_case_completed("goal_r3_provider_handoff_preserves_usd_budget") {
            return;
        }
        for limit in [1.0, 0.05, 0.09] {
            check_goal_usd_budget(
                false,
                BudgetCompletion::DuringResponse,
                "end_turn",
                limit,
                false,
            )
            .await;
        }
    }

    #[tokio::test]
    async fn goal_r3_direct_handoff_preserves_usd_budget() {
        if isolated_case_completed("goal_r3_direct_handoff_preserves_usd_budget") {
            return;
        }
        for limit in [1.0, 0.05, 0.09] {
            check_goal_usd_budget(
                true,
                BudgetCompletion::DuringResponse,
                "end_turn",
                limit,
                false,
            )
            .await;
        }
    }

    async fn check_cancelled_goal_handoff_matrix(direct: bool) {
        for (completion, limit, cancel_response) in [
            (BudgetCompletion::Active, 0.05, true),
            (BudgetCompletion::DuringResponse, 1.0, true),
            (BudgetCompletion::Active, 0.05, false),
            (BudgetCompletion::DuringResponse, 0.05, false),
            (BudgetCompletion::DuringResponse, 0.05, true),
            (BudgetCompletion::DuringResponse, 0.09, true),
        ] {
            check_goal_usd_budget(
                direct,
                completion,
                if cancel_response {
                    "tool_use"
                } else {
                    "end_turn"
                },
                limit,
                cancel_response,
            )
            .await;
        }
    }

    #[tokio::test]
    async fn goal_r4_provider_cancelled_handoff_preserves_caller_outcome() {
        if isolated_case_completed("goal_r4_provider_cancelled_handoff_preserves_caller_outcome") {
            return;
        }
        check_cancelled_goal_handoff_matrix(false).await;
    }

    #[tokio::test]
    async fn goal_r4_direct_cancelled_handoff_preserves_caller_outcome() {
        if isolated_case_completed("goal_r4_direct_cancelled_handoff_preserves_caller_outcome") {
            return;
        }
        check_cancelled_goal_handoff_matrix(true).await;
    }

    #[tokio::test]
    async fn housekeeping_memory_requests_cannot_use_foreground_completion() {
        if isolated_case_completed("housekeeping_memory_requests_cannot_use_foreground_completion")
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let context = GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
        let background = context.for_background_work();
        let (base, count, stop, server) =
            serve_goal_responses(|_| sse_response("[]", 1, true)).await;
        let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
            api_key: "test-key".into(),
            api_base: base,
            max_retries: 0,
            ..Default::default()
        })
        .unwrap()
        .for_goal_turn(background.captured().unwrap(), GoalTurnPermission::Work);
        let extractor = session_memory::SessionMemoryExtractor::new("claude-test");
        let history: Vec<_> = (0..20)
            .map(|n| Message::user(format!("Memory {n}")))
            .collect();
        assert!(session_memory::SessionMemoryExtractor::should_extract(
            &history
        ));
        assert!(extractor
            .extract(&history, dir.path(), &client)
            .await
            .is_ok());
        store
            .complete_goal_turn(context.captured().unwrap())
            .unwrap();
        assert!(extractor
            .extract(&history, dir.path(), &client)
            .await
            .is_err());
        assert!(
            run_goal_fixture(dir.path(), background, None, GoalFixture::default())
                .await
                .requests
                .is_empty()
        );
        // Make AutoDream eligible. A final summary must return before even
        // acquiring its lock or spawning the detached AgentTool.
        let home = claurst_core::config::config_home();
        std::fs::create_dir_all(home.join("conversations")).unwrap();
        for n in 0..5 {
            std::fs::write(
                home.join("conversations").join(format!("{n}.jsonl")),
                "fixture",
            )
            .unwrap();
        }
        let foreground = run_goal_fixture(
            dir.path(),
            context,
            None,
            GoalFixture {
                initial_history: Some(history),
                ..Default::default()
            },
        )
        .await;
        stop.cancel();
        server.await.unwrap();
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(matches!(foreground.outcome, QueryOutcome::EndTurn { .. }));
        assert_eq!(foreground.requests.len(), 1);
        assert!(!home.join("memory/.consolidation_lock").exists());
        assert!(!home.join("memory/.consolidation_state.json").exists());
    }

    #[tokio::test]
    async fn raw_ping_stream_revocation_stops_before_eof_or_stall() {
        if isolated_case_completed("raw_ping_stream_revocation_stops_before_eof_or_stall") {
            return;
        }
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        GoalStore::open(&path)
            .unwrap()
            .set_goal("goal-query", "A", None)
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hook_path = path.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let server_stop = stop.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            assert!(socket.read(&mut buffer).await.unwrap() > 0);
            // Keep the body open after Ping: neither EOF nor MessageStop can
            // accidentally terminate the test before the ownership watcher.
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 999999\r\n\r\nevent: ping\ndata: {}\n\n").await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            GoalStore::open(&hook_path)
                .unwrap()
                .set_status("goal-query", GoalStatus::Paused)
                .unwrap();
            server_stop.cancelled().await;
        });
        let run = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            run_goal_fixture(
                dir.path(),
                GoalTurnContext::Pending(path),
                None,
                GoalFixture {
                    direct_api_base: Some(base),
                    ..Default::default()
                },
            ),
        )
        .await;
        stop.cancel();
        server.await.unwrap();
        assert!(matches!(run.unwrap().outcome, QueryOutcome::Error(_)));
    }

    #[cfg(unix)]
    #[test]
    fn queued_stop_hook_rechecks_goal_before_child_start() {
        if isolated_case_completed("queued_stop_hook_rechecks_goal_before_child_start") {
            return;
        }
        for revoke in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("goals.sqlite");
            let store = GoalStore::open(&path).unwrap();
            store.set_goal("goal-query", "A", None).unwrap();
            let context =
                GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .max_blocking_threads(1)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
                let (release_tx, release_rx) = std::sync::mpsc::channel();
                let blocker = tokio::task::spawn_blocking(move || {
                    entered_tx.send(()).unwrap();
                    release_rx
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .unwrap();
                });
                entered_rx.await.unwrap();
                let mut config = Config::default();
                config.hooks.insert(
                    HookEvent::Stop,
                    vec![HookEntry {
                        command: "printf started > stop-hook.txt".into(),
                        tool_filter: None,
                        blocking: false,
                    }],
                );
                let (reaper_tx, reaper_rx) = std::sync::mpsc::channel();
                stop_hooks_with_reaper_observer(
                    &Message::user("Done."),
                    &config,
                    dir.path().to_path_buf(),
                    context.for_background_work().captured().cloned(),
                    Some(reaper_tx),
                );
                if revoke {
                    store.set_status("goal-query", GoalStatus::Paused).unwrap();
                }
                release_tx.send(()).unwrap();
                blocker.await.unwrap();
                let reaper = reaper_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .expect("Stop hook reaper was spawned");
                let hook_path = dir.path().join("stop-hook.txt");
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    let started = hook_path.exists();
                    if started || reaper.is_finished() {
                        break;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        panic!("Stop hook reaper did not finish before deadline");
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    tokio::task::spawn_blocking(move || reaper.join()),
                )
                .await
                .expect("Stop hook reaper join timed out")
                .expect("Stop hook reaper join task failed")
                .expect("Stop hook reaper panicked");
                assert_eq!(hook_path.exists(), !revoke, "hook state after reaper");
            });
            runtime.shutdown_timeout(std::time::Duration::from_secs(1));
        }
    }

    #[tokio::test]
    async fn every_captured_stop_rejects_dispatch_and_stays_revoked_on_reentry() {
        if isolated_case_completed(
            "every_captured_stop_rejects_dispatch_and_stays_revoked_on_reentry",
        ) {
            return;
        }
        for stop in [
            "pause",
            "complete",
            "budget",
            "clear",
            "replace",
            "store-error",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("goals.sqlite");
            let store = GoalStore::open(&path).unwrap();
            store.set_goal("goal-query", "A", Some(100)).unwrap();
            let context =
                GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
            match stop {
                "pause" => store.set_status("goal-query", GoalStatus::Paused).unwrap(),
                "complete" => store
                    .set_status("goal-query", GoalStatus::Complete)
                    .unwrap(),
                "budget" => store
                    .set_status("goal-query", GoalStatus::BudgetLimited)
                    .unwrap(),
                "clear" => store.clear_goal("goal-query").unwrap(),
                "replace" => {
                    store.set_goal("goal-query", "B", Some(900)).unwrap();
                    store.record_completed_turn("goal-query", 18, 7).unwrap();
                }
                "store-error" => {
                    std::fs::rename(&path, dir.path().join("unavailable.sqlite")).unwrap();
                }
                _ => unreachable!(),
            }
            let before = store.try_get_goal("goal-query").unwrap();
            let run =
                run_goal_fixture(dir.path(), context.clone(), None, GoalFixture::default()).await;
            assert!(matches!(run.outcome, QueryOutcome::Error(_)), "{stop}");
            assert!(run.requests.is_empty(), "{stop}");
            assert_eq!(store.try_get_goal("goal-query").unwrap(), before, "{stop}");
            if stop == "store-error" {
                std::fs::rename(dir.path().join("unavailable.sqlite"), &path).unwrap();
            }
            store.set_goal("goal-query", "Later B", Some(700)).unwrap();
            let before = store.try_get_goal("goal-query").unwrap();
            let again = run_goal_fixture(dir.path(), context, None, GoalFixture::default()).await;
            assert!(again.requests.is_empty(), "{stop} reentry");
            assert_eq!(store.try_get_goal("goal-query").unwrap(), before);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_cli_stale_resume_revalidates_before_fresh_spawn() {
        if isolated_case_completed("claude_cli_stale_resume_revalidates_before_fresh_spawn") {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        for revoke in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("goals.sqlite");
            let store = GoalStore::open(&path).unwrap();
            store.set_goal("goal-query", "A", None).unwrap();
            let turn = store.capture_active_goal("goal-query").unwrap().unwrap();
            let binary = dir.path().join("fake-claude");
            std::fs::write(&binary, r#"#!/usr/bin/env python3
import json, os, pathlib, sqlite3, sys
root = pathlib.Path(__file__).parent
with (root / 'spawns.jsonl').open('a') as out:
    out.write(json.dumps({'pid': os.getpid(), 'resumed': '--resume' in sys.argv, 'argv': sys.argv[1:]}) + '\n')
sys.stdin.readline()
if '--resume' in sys.argv:
    if (root / 'revoke').exists():
        db = sqlite3.connect(root / 'goals.sqlite')
        db.execute("UPDATE goals SET status='paused'")
        db.commit()
        db.close()
    sys.exit(0)
print(json.dumps({'type':'system','session_id':'stale-fixture','model':'claude-test'}), flush=True)
print(json.dumps({'type':'assistant','message':{'content':[{'type':'text','text':'Done.'}]}}), flush=True)
print(json.dumps({'type':'result','is_error':False,'usage':{'input_tokens':1,'output_tokens':1}}), flush=True)
"#).unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
            if revoke {
                std::fs::write(dir.path().join("revoke"), "pause").unwrap();
            }
            // Only this exact child test sees the fake executable. Its own
            // application homes and Claude config were isolated before entry.
            let prior = std::env::var_os("COVEN_CODE_CLAUDE_BIN");
            std::env::set_var("COVEN_CODE_CLAUDE_BIN", &binary);
            let provider = claurst_api::providers::claude_cli::ClaudeCliProvider::new();
            match prior {
                Some(value) => std::env::set_var("COVEN_CODE_CLAUDE_BIN", value),
                None => std::env::remove_var("COVEN_CODE_CLAUDE_BIN"),
            }
            let mut request = ProviderRequest {
                model: "claude-test".into(),
                messages: vec![Message::user("Prime session")],
                system_prompt: None,
                tools: vec![],
                max_tokens: 100,
                temperature: None,
                top_p: None,
                top_k: None,
                stop_sequences: vec![],
                thinking: None,
                provider_options: serde_json::json!({}),
            };
            let prime = provider
                .create_message_stream_for_goal(
                    request.clone(),
                    Some(&turn),
                    GoalTurnPermission::Work,
                )
                .await
                .unwrap();
            let events: Vec<_> =
                tokio::time::timeout(std::time::Duration::from_secs(2), prime.collect())
                    .await
                    .unwrap();
            assert!(events
                .iter()
                .any(|e| matches!(e, Ok(StreamEvent::MessageStop))));
            request.messages.push(Message {
                role: Role::Assistant,
                content: MessageContent::Text("Earlier reply".into()),
                uuid: None,
                cost: None,
                snapshot_patch: None,
            });
            request.messages.push(Message::user("Continue A"));
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                provider.create_message_stream_for_goal(
                    request,
                    Some(&turn),
                    GoalTurnPermission::Work,
                ),
            )
            .await
            .unwrap();
            let failed = match response {
                Err(_) => true,
                Ok(stream) => {
                    let events: Vec<_> =
                        tokio::time::timeout(std::time::Duration::from_secs(2), stream.collect())
                            .await
                            .unwrap();
                    assert!(events
                        .iter()
                        .any(|e| matches!(e, Ok(StreamEvent::MessageStop))));
                    false
                }
            };
            let spawns: Vec<Value> = std::fs::read_to_string(dir.path().join("spawns.jsonl"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            // A priming request, then stale resume, then an active-only fresh
            // fallback. Every fixture process must already have been reaped.
            for spawn in &spawns {
                assert!(!spawn["argv"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|arg| arg == "--tools"));
                let status = std::process::Command::new("/bin/kill")
                    .args(["-0", &spawn["pid"].as_u64().unwrap().to_string()])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap();
                assert!(
                    !status.success(),
                    "owned fake CLI child remained alive: {spawn}"
                );
            }
            assert_eq!(spawns.len(), if revoke { 2 } else { 3 });
            assert_eq!(failed, revoke);
            assert_eq!(spawns[1]["resumed"], true);
            if !revoke {
                assert_eq!(spawns[2]["resumed"], false);
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_cli_final_response_disables_native_and_mcp_tools() {
        if isolated_case_completed("claude_cli_final_response_disables_native_and_mcp_tools") {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = GoalStore::open(&dir.path().join("goals.sqlite")).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let turn = store.capture_active_goal("goal-query").unwrap().unwrap();
        store.complete_goal_turn(&turn).unwrap();
        let binary = dir.path().join("fake-claude");
        std::fs::write(
            &binary,
            r#"#!/usr/bin/env python3
import json, pathlib, sys
root = pathlib.Path(__file__).parent
(root / 'argv.json').write_text(json.dumps(sys.argv[1:]))
sys.stdin.readline()
print(json.dumps({'type':'result','is_error':False}), flush=True)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let prior = std::env::var_os("COVEN_CODE_CLAUDE_BIN");
        std::env::set_var("COVEN_CODE_CLAUDE_BIN", &binary);
        let provider = claurst_api::providers::claude_cli::ClaudeCliProvider::new();
        match prior {
            Some(value) => std::env::set_var("COVEN_CODE_CLAUDE_BIN", value),
            None => std::env::remove_var("COVEN_CODE_CLAUDE_BIN"),
        }
        let request = ProviderRequest {
            model: "claude-test".into(),
            messages: vec![Message::user("Final summary")],
            system_prompt: None,
            tools: vec![],
            max_tokens: 100,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: vec![],
            thinking: None,
            provider_options: serde_json::json!({}),
        };
        let stream = provider
            .create_message_stream_for_goal(request, Some(&turn), GoalTurnPermission::FinalResponse)
            .await
            .unwrap();
        let events: Vec<_> =
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.collect())
                .await
                .unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, Ok(StreamEvent::MessageStop))));
        let argv: Vec<String> =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("argv.json")).unwrap())
                .unwrap();
        assert!(argv.iter().any(|arg| arg == "--safe-mode"));
        assert!(argv.windows(2).any(|args| args == ["--tools", ""]));
        assert!(argv
            .windows(2)
            .any(|args| args == ["--disallowedTools", "*"]));
    }
    #[tokio::test]
    async fn goal_r1_concurrent_final_dispatch_is_reserved_and_cancel_releases() {
        if isolated_case_completed(
            "goal_r1_concurrent_final_dispatch_is_reserved_and_cancel_releases",
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = GoalStore::open(&dir.path().join("goals.sqlite")).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let turn = store.capture_active_goal("goal-query").unwrap().unwrap();
        store.complete_goal_turn(&turn).unwrap();
        let context = GoalTurnContext::Captured(Some(turn));
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel = tokio_util::sync::CancellationToken::new();
        let fixture = || {
            let count = count.clone();
            GoalFixture {
                pending_creation: true,
                cancel_token: Some(cancel.clone()),
                on_request: Some(Arc::new(move |_| {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })),
                ..Default::default()
            }
        };
        let (a, b, ()) = tokio::join!(
            run_goal_fixture(dir.path(), context.clone(), None, fixture()),
            run_goal_fixture(dir.path(), context.clone(), None, fixture()),
            async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                cancel.cancel();
            }
        );
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(
            matches!(a.outcome, QueryOutcome::Cancelled)
                || matches!(b.outcome, QueryOutcome::Cancelled)
        );
        let retry = run_goal_fixture(dir.path(), context, None, GoalFixture::default()).await;
        assert!(
            matches!(retry.outcome, QueryOutcome::EndTurn { .. }),
            "{:?}",
            retry.outcome
        );
        assert_eq!(retry.requests.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn goal_r1_stop_hook_does_not_hold_runtime_shutdown_and_is_reaped() {
        if isolated_case_completed("goal_r1_stop_hook_does_not_hold_runtime_shutdown_and_is_reaped")
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("hook.pid");
        let mut config = Config::default();
        config.hooks.insert(
            HookEvent::Stop,
            vec![HookEntry {
                command: format!("echo $$ > '{}'; exec sleep 1", marker.display()),
                ..Default::default()
            }],
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            stop_hooks_with_full_behavior(
                &Message::user("Done"),
                &config,
                dir.path().to_owned(),
                None,
            );
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while !marker.exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                }
            })
            .await
            .unwrap();
        });
        let start = std::time::Instant::now();
        drop(runtime);
        let elapsed = start.elapsed();
        let pid = std::fs::read_to_string(marker).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let reaped = loop {
            if !Command::new("/bin/kill")
                .args(["-0", pid.trim()])
                .output()
                .unwrap()
                .status
                .success()
            {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(reaped, "owned hook child was not reaped");
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "runtime shutdown blocked {elapsed:?}"
        );
    }

    fn r1_context(
        root: &Path,
        session: &str,
        turn: GoalTurnContext,
        config: Config,
    ) -> ToolContext {
        ToolContext {
            working_dir: root.to_owned(),
            permission_mode: PermissionMode::BypassPermissions,
            permission_handler: Arc::new(AutoPermissionHandler {
                mode: PermissionMode::BypassPermissions,
            }),
            cost_tracker: CostTracker::new(),
            session_id: session.into(),
            goal_turn: turn,
            file_history: Default::default(),
            current_turn: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: None,
            config,
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
        }
    }

    fn r1_sse(block: Value, stop: &str) -> String {
        use serde_json::json;
        let mut lines = vec![
            json!({"type":"message_start","message":{"id":"local","type":"message","role":"assistant","model":"claude-test","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":block}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":stop},"usage":{"output_tokens":1}}),
            json!({"type":"message_stop"}),
        ];
        if lines[1]["content_block"]["name"] == "GoalComplete" {
            lines.insert(1, json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":"Child findings preserved."}}));
            lines.insert(2, json!({"type":"content_block_stop","index":1}));
        }
        lines
            .drain(..)
            .map(|value| {
                format!(
                    "event: {}\ndata: {}\n\n",
                    value["type"].as_str().unwrap(),
                    value
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn goal_r1_actual_agent_preserves_foreground_summary() {
        if isolated_case_completed("goal_r1_actual_agent_preserves_foreground_summary") {
            return;
        }
        use claurst_api::AnthropicClient;
        use serde_json::json;
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let store = GoalStore::open(&root.join("agent.sqlite")).unwrap();
        store.set_goal("agent", "Complete A", None).unwrap();
        let turn = GoalTurnContext::Captured(store.capture_active_goal("agent").unwrap());
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server_requests = requests.clone();
        let server_stop = stop.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(12);
            while !server_stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                let (mut socket, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut raw = Vec::new();
                let mut byte = [0u8; 1];
                while !raw.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).unwrap();
                    raw.push(byte[0]);
                }
                let headers = String::from_utf8(raw).unwrap();
                let len: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body = vec![0; len];
                socket.read_exact(&mut body).unwrap();
                let n = {
                    let mut requests = server_requests.lock().unwrap();
                    requests.push(serde_json::from_slice(&body).unwrap());
                    requests.len()
                };
                let body = match n {
                    1 => r1_sse(
                        json!({"type":"tool_use","id":"delegate","name":claurst_core::constants::TOOL_NAME_AGENT,"input":{"description":"Local completion fixture","prompt":"Complete the disposable goal","tools":["GoalComplete"],"model":"claude-test","max_turns":5}}),
                        "tool_use",
                    ),
                    2 => r1_sse(
                        json!({"type":"tool_use","id":"complete","name":"GoalComplete","input":{"audit_summary":"done","evidence":"fixture assertion"}}),
                        "tool_use",
                    ),
                    _ => r1_sse(
                        json!({"type":"text","text":"Foreground completed A."}),
                        "end_turn",
                    ),
                };
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        let mut config = Config {
            api_key: Some("local-fixture".into()),
            ..Default::default()
        };
        config.provider_configs.insert(
            "anthropic".into(),
            serde_json::from_value(
                json!({"api_base":base,"api_key":"local-fixture","enabled":true}),
            )
            .unwrap(),
        );
        let ctx = r1_context(root, "agent", turn, config);
        let cfg = QueryConfig {
            model: "claude-test".into(),
            system_prompt: Some("Local fixture".into()),
            max_turns: 5,
            ..Default::default()
        };
        let client = AnthropicClient::new(claurst_api::client::ClientConfig {
            api_key: "local-fixture".into(),
            api_base: base,
            max_retries: 0,
            ..Default::default()
        })
        .unwrap();
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(AgentTool)];
        let mut messages = vec![Message::user("Delegate then summarize to user")];
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            run_query_loop(
                &client,
                &mut messages,
                &tools,
                &ctx,
                &cfg,
                ctx.cost_tracker.clone(),
                Some(tx),
                Default::default(),
                None,
            ),
        )
        .await;
        stop.store(true, Ordering::SeqCst);
        server.join().unwrap();
        let outcome = outcome.expect("bounded AgentTool fixture");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(requests.lock().unwrap().len(), 3);
        let final_request = requests.lock().unwrap().last().unwrap().clone();
        assert!(
            final_request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message.to_string().contains("delegate")),
            "summary must belong to foreground history"
        );
        assert!(final_request["messages"]
            .to_string()
            .contains("Child findings preserved."));
        assert!(final_request["messages"]
            .to_string()
            .contains("Audit summary: done"));
        assert!(final_request["messages"]
            .to_string()
            .contains("Evidence: fixture assertion"));
        assert!(
            matches!(outcome, QueryOutcome::EndTurn { .. }),
            "foreground outcome: {outcome:?}"
        );
        assert_eq!(
            store.try_get_goal("agent").unwrap().unwrap().status,
            claurst_core::GoalStatus::Complete
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, QueryEvent::TurnComplete { .. })),
            "no user-facing completed response"
        );
        assert!(messages
            .iter()
            .any(|message| format!("{:?}", message).contains("Foreground completed A.")));
    }

    #[tokio::test]
    async fn goal_r1_background_child_completion_during_foreground_request_keeps_summary() {
        if isolated_case_completed(
            "goal_r1_background_child_completion_during_foreground_request_keeps_summary",
        ) {
            return;
        }
        use serde_json::json;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("goal-query", "A", None).unwrap();
        let context = GoalTurnContext::Captured(store.capture_active_goal("goal-query").unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = tokio_util::sync::CancellationToken::new();
        let done = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let recorded = requests.clone();
        let parent_waiting = Arc::new(tokio::sync::Notify::new());
        let waiting = parent_waiting.clone();
        let server_path = path.clone();
        let server = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (mut socket, _) = tokio::select! { _ = done.cancelled() => break, result = listener.accept() => result.unwrap() };
                let requests = recorded.clone();
                let waiting = waiting.clone();
                let path = server_path.clone();
                connections.spawn(async move {
                    let mut raw = Vec::new();
                    let body = loop {
                        let mut buffer = [0u8; 4096];
                        let n = socket.read(&mut buffer).await.unwrap();
                        assert!(n > 0);
                        raw.extend_from_slice(&buffer[..n]);
                        if let Some(end) = raw.windows(4).position(|s| s == b"\r\n\r\n") {
                            let headers = std::str::from_utf8(&raw[..end]).unwrap();
                            let len = headers.lines().find_map(|s| s.to_ascii_lowercase().strip_prefix("content-length:").map(|n| n.trim().parse::<usize>().unwrap())).unwrap();
                            if raw.len() >= end + 4 + len { break serde_json::from_slice::<Value>(&raw[end+4..end+4+len]).unwrap(); }
                        }
                    };
                    let child = body["system"].to_string().contains("specialized AI agent");
                    let is_final = body["tools"].as_array().is_none_or(|tools| tools.is_empty());
                    let first = { let mut requests = requests.lock().unwrap(); requests.push(body); requests.len() == 1 };
                    let body = if first {
                        r1_sse(json!({"type":"tool_use","id":"delegate-background","name":"Agent","input":{"description":"Local background completion", "prompt":"Complete the disposable goal", "tools":["GoalComplete"],"model":"claude-test", "run_in_background":true}}), "tool_use")
                    } else if child {
                        tokio::time::timeout(std::time::Duration::from_secs(3), waiting.notified()).await.unwrap();
                        r1_sse(json!({"type":"tool_use","id":"complete","name":"GoalComplete","input":{"audit_summary":"done","evidence":"local background proof"}}), "tool_use")
                    } else if !is_final {
                        waiting.notify_one();
                        tokio::time::timeout(std::time::Duration::from_secs(3), async {
                            while GoalStore::open(&path).unwrap().try_get_goal("goal-query").unwrap().unwrap().status != GoalStatus::Complete {
                                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                            }
                        }).await.unwrap();
                        r1_sse(json!({"type":"text","text":"Already received foreground output."}), "end_turn")
                    } else {
                        r1_sse(json!({"type":"text","text":"Foreground background-completion summary."}), "end_turn")
                    };
                    let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
                });
            }
            while let Some(result) = connections.join_next().await {
                result.unwrap();
            }
        });
        let mut config = Config {
            api_key: Some("local-fixture".into()),
            ..Default::default()
        };
        config.provider_configs.insert(
            "anthropic".into(),
            serde_json::from_value(
                json!({"api_base":base,"api_key":"local-fixture","enabled":true}),
            )
            .unwrap(),
        );
        let ctx = r1_context(dir.path(), "goal-query", context, config);
        let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
            api_key: "local-fixture".into(),
            api_base: base,
            max_retries: 0,
            ..Default::default()
        })
        .unwrap();
        let config = QueryConfig {
            model: "claude-test".into(),
            system_prompt: Some("Local foreground fixture".into()),
            max_turns: 5,
            ..Default::default()
        };
        let mut messages = vec![Message::user("Delegate then summarize")];
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(AgentTool)];
        let (tx, mut rx) = mpsc::unbounded_channel();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            run_query_loop(
                &client,
                &mut messages,
                &tools,
                &ctx,
                &config,
                ctx.cost_tracker.clone(),
                Some(tx),
                Default::default(),
                None,
            ),
        )
        .await;
        stop.cancel();
        server.await.unwrap();
        let outcome = result.unwrap();
        assert!(
            matches!(&outcome, QueryOutcome::EndTurn { message, .. } if message.get_all_text() == "Foreground background-completion summary."),
            "{outcome:?}"
        );
        assert!(std::iter::from_fn(|| rx.try_recv().ok())
            .any(|event| matches!(event, QueryEvent::TurnComplete { .. })));
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request["system"]
                    .to_string()
                    .contains("specialized AI agent"))
                .count(),
            1,
            "child must not dispatch a summary"
        );
    }
}
