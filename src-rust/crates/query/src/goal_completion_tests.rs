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
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test_name, "--nocapture", "--test-threads=1"])
            .current_dir(fixture.path())
            .env("COVEN_CODE_GOAL_QUERY_TEST", &test_name)
            .env("COVEN_CODE_GOAL_QUERY_HOME", &home)
            .env("COVEN_CODE_HOME", &home)
            .env("COVEN_HOME", &coven_home)
            .env("COVEN_CODE_GOALS", "1")
            .env("COVEN_CODE_DISABLE_MODELS_FETCH", "1")
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

    struct RecordingGoalProvider {
        inner: ScriptedProvider,
        requests: Arc<Mutex<Vec<ProviderRequest>>>,
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
            self.requests.lock().unwrap().push(request.clone());
            self.inner.create_message_stream(request).await
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            self.inner.list_models().await
        }
        async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
            self.inner.health_check().await
        }
        fn capabilities(&self) -> ProviderCapabilities {
            self.inner.capabilities()
        }
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
    }

    async fn run_goal_query(
        root: &Path,
        goal_turn: GoalTurnContext,
        replace: bool,
        initial_message: Option<String>,
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
        let requests = Arc::new(Mutex::new(Vec::new()));
        let replacement = Arc::new(Mutex::new(None));
        let mut registry = claurst_api::ProviderRegistry::new();
        registry.register(Arc::new(RecordingGoalProvider {
            inner: ScriptedProvider::new(rounds),
            requests: requests.clone(),
        }));
        let mut config = make_config(Some("Local test"), Some("Keep existing instruction"));
        config.model = "scripted".to_string();
        config.provider_registry = Some(Arc::new(registry));
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
            Box::new(ReplaceGoalFixture {
                path: root.join("goals.sqlite"),
                replacement: replacement.clone(),
            }),
        ];
        let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
            api_key: "unused-test-key".to_string(),
            ..Default::default()
        })
        .unwrap();
        let mut messages = vec![Message::user(
            initial_message.unwrap_or_else(|| "Finish A".to_string()),
        )];
        let (tx, mut rx) = mpsc::unbounded_channel();
        let outcome = run_query_loop(
            &client,
            &mut messages,
            &tools,
            &ctx,
            &config,
            tracker,
            Some(tx),
            tokio_util::sync::CancellationToken::new(),
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
        assert_completion_result(&run, true);
        assert_original_prompt(&run);
        assert_eq!(store.try_get_goal("goal-query").unwrap(), run.replacement);
        assert!(run.events.iter().any(|event| matches!(event,
            QueryEvent::ToolEnd { tool_id, result, .. }
                if tool_id == "complete-a" && result.contains("Goal was replaced"))));
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
        assert_completion_result(&run, true);
        assert_original_prompt(&run);
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
    async fn capture_store_failure_denies_completion_but_allows_final_reply() {
        if isolated_case_completed("capture_store_failure_denies_completion_but_allows_final_reply")
        {
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
        assert_completion_result(&run, true);
        assert!(run.events.iter().any(|event| matches!(event,
            QueryEvent::Status(text) if text.contains("could not capture turn") && text.contains("Goal DB error"))));
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
}
