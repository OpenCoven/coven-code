// Included in the existing query test module to exercise its provider fixtures.
#[tokio::test]
async fn stop_after_eleven_rounds_keeps_completed_tool_results_and_partial_output() {
    struct StopAtEleven {
        cancel: tokio_util::sync::CancellationToken,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }
    #[async_trait]
    impl Tool for StopAtEleven {
        fn name(&self) -> &str {
            "StopFixture"
        }
        fn description(&self) -> &str {
            "Signals the same cancellation token as Stop"
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::None
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({"type":"object"})
        }
        async fn execute(&self, _: Value, ctx: &ToolContext) -> ToolResult {
            let count = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            // A child tool can update this display/history counter. It must not
            // affect the query's own model-round receipt.
            ctx.current_turn
                .store(999, std::sync::atomic::Ordering::SeqCst);
            if count == 11 {
                self.cancel.cancel();
            }
            ToolResult::success(format!("completed side effect {count}"))
        }
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(StopAtEleven {
        cancel: cancel.clone(),
        calls: calls.clone(),
    })];
    // Round twelve is deliberately available: Stop must prevent its invocation.
    let rounds = (1..=12)
        .map(|n| {
            scripted_round(
                &format!("stop-{n}"),
                vec![
                    ContentBlock::Text {
                        text: format!("partial output {n}"),
                    },
                    ContentBlock::ToolUse {
                        id: format!("stop-tool-{n}"),
                        name: "StopFixture".into(),
                        input: serde_json::json!({}),
                    },
                ],
                ProviderStopReason::ToolUse,
            )
        })
        .collect();
    let (outcome, events, messages, _) = run_scripted_provider_with_token(
        ScriptedProvider::new(rounds),
        QueryConfig {
            max_turns: 12,
            ..make_config(None, None)
        },
        Config {
            provider: Some(SCRIPTED_PROVIDER_ID.into()),
            ..Config::default()
        },
        &tools,
        cancel,
    )
    .await;
    assert!(matches!(outcome, QueryOutcome::Cancelled));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 11);
    assert_eq!(
        messages
            .iter()
            .filter(|message| is_tool_result_carrier(message))
            .count(),
        11
    );
    assert!(messages
        .iter()
        .any(|message| message.get_all_text() == "partial output 1"));
    assert!(events
        .iter()
        .all(|event| !matches!(event, QueryEvent::TurnComplete { .. })));
    let transcript = serde_json::to_string(&messages).unwrap();
    for n in 1..=11 {
        assert!(transcript.contains(&format!("completed side effect {n}")));
    }
}

#[tokio::test]
async fn model_round_receipt_does_not_read_the_shared_tool_display_counter() {
    struct CounterTool;
    #[async_trait]
    impl Tool for CounterTool {
        fn name(&self) -> &str {
            "CounterFixture"
        }
        fn description(&self) -> &str {
            "Emulates a child updating the tool display counter"
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::None
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({"type":"object"})
        }
        async fn execute(&self, _: Value, ctx: &ToolContext) -> ToolResult {
            ctx.current_turn
                .store(999, std::sync::atomic::Ordering::SeqCst);
            ToolResult::success("retained child result")
        }
    }
    let provider = ScriptedProvider::new(vec![scripted_round(
        "counter",
        vec![ContentBlock::ToolUse {
            id: "counter-tool".into(),
            name: "CounterFixture".into(),
            input: serde_json::json!({}),
        }],
        ProviderStopReason::ToolUse,
    )]);
    let temp = tempfile::tempdir().unwrap();
    let mut registry = claurst_api::ProviderRegistry::new();
    registry.register(Arc::new(provider));
    let config = QueryConfig {
        model: format!("{SCRIPTED_PROVIDER_ID}/scripted"),
        max_turns: 1,
        provider_registry: Some(Arc::new(registry)),
        ..Default::default()
    };
    let tracker = CostTracker::new();
    let context = ToolContext {
        working_dir: temp.path().to_owned(),
        permission_mode: PermissionMode::BypassPermissions,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::BypassPermissions,
        }),
        cost_tracker: tracker.clone(),
        session_id: "budget-counter".into(),
        goal_turn: Default::default(),
        file_history: Default::default(),
        current_turn: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        non_interactive: true,
        mcp_manager: None,
        config: Config {
            provider: Some(SCRIPTED_PROVIDER_ID.into()),
            ..Default::default()
        },
        managed_agent_config: None,
        completion_notifier: None,
        pending_permissions: None,
        permission_manager: None,
        user_question_tx: None,
    };
    let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
        api_key: "unused".into(),
        ..Default::default()
    })
    .unwrap();
    let run = run_query_loop_with_receipt(
        &client,
        &mut vec![Message::user("count")],
        &[Box::new(CounterTool)],
        &context,
        &config,
        tracker,
        None,
        tokio_util::sync::CancellationToken::new(),
        None,
    )
    .await;
    assert_eq!(
        context
            .current_turn
            .load(std::sync::atomic::Ordering::SeqCst),
        999
    );
    assert_eq!(run.num_turns, 1);
    assert!(matches!(
        run.outcome,
        QueryOutcome::TurnBudgetExhausted { consumed: 1, .. }
    ));
}
