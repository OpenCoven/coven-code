use super::*;

// Every owning test re-execs in an isolated process: App and Settings must not
// consult installed credentials, model caches, a daemon, or a real Claude CLI.
fn isolated(name: &str) -> bool {
    let name = format!("goal_dispatch_tests::{name}");
    if std::env::var("COVEN_CODE_GOAL_DISPATCH_TEST").as_deref() == Ok(&name) {
        return false;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("code");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("auth.json"), r#"{"credentials":{}}"#).unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        let text = key.to_string_lossy();
        if text.contains("API_KEY")
            || text.starts_with("COVEN") && (text.contains("TEST") || text.contains("FIXTURE"))
        {
            command.env_remove(key);
        }
    }
    let mut child = command
        .args(["--exact", &name, "--nocapture", "--test-threads=1"])
        .env("COVEN_CODE_GOAL_DISPATCH_TEST", &name)
        .env("COVEN_CODE_HOME", &home)
        .env("COVEN_HOME", dir.path().join("coven"))
        .env("CLAUDE_CONFIG_DIR", dir.path().join("claude"))
        .env("COVEN_CODE_CLAUDE_BIN", dir.path().join("absent-claude"))
        .env("COVEN_DAEMON_SOCKET", dir.path().join("absent.sock"))
        .env("COVEN_CODE_DISABLE_MODELS_FETCH", "1")
        .stdout(std::fs::File::create(dir.path().join("out")).unwrap())
        .stderr(std::fs::File::create(dir.path().join("err")).unwrap())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("isolated dispatch fixture timed out: {name}");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let out = std::fs::read_to_string(dir.path().join("out")).unwrap();
    let err = std::fs::read_to_string(dir.path().join("err")).unwrap();
    assert!(
        status.success() && out.contains("1 passed; 0 failed"),
        "{out}\n{err}"
    );
    true
}

#[test]
fn goal_dispatch_recovery_blocks_actual_idle_selector() {
    if isolated("goal_dispatch_recovery_blocks_actual_idle_selector") {
        return;
    }
    let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
    app.auto_compact_enabled = true;
    app.context_window_size = 100;
    app.context_used_tokens = 99;
    app.auto_compact_threshold = 90;
    assert_eq!(select_idle_compaction(&app, false), Some(99));
    app.open_rate_limit_recovery("limited".into(), Some(60));
    assert_eq!(
        select_idle_compaction(&app, false),
        None,
        "visible recovery must own idle time"
    );
    app.rate_limit_recovery.request_retry(None);
    assert_eq!(
        select_idle_compaction(&app, false),
        None,
        "pending retry must own idle time"
    );
    assert!(app.rate_limit_recovery.take_retry_directive().is_some());
    assert_eq!(select_idle_compaction(&app, false), Some(99));
}

fn context(path: PathBuf) -> ToolContext {
    ToolContext {
        working_dir: path.parent().unwrap().to_path_buf(),
        permission_mode: PermissionMode::BypassPermissions,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::BypassPermissions,
        }),
        cost_tracker: CostTracker::new(),
        session_id: "dispatch".into(),
        goal_turn: claurst_core::goal::GoalTurnContext::Pending(path),
        file_history: Default::default(),
        current_turn: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        non_interactive: true,
        mcp_manager: None,
        config: Config::default(),
        managed_agent_config: None,
        completion_notifier: None,
        pending_permissions: None,
        permission_manager: None,
        user_question_tx: None,
    }
}

type HttpFixture = (
    Arc<claurst_api::AnthropicClient>,
    Arc<std::sync::atomic::AtomicUsize>,
    Arc<std::sync::Mutex<Vec<String>>>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
);

#[derive(Clone, Copy)]
enum Reply {
    Limited,
    Partial,
    Complete,
}

async fn rate_limit_server() -> HttpFixture {
    scripted_server(vec![Reply::Limited]).await
}

async fn scripted_server(replies: Vec<Reply>) -> HttpFixture {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = requests.clone();
    let models = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed_models = models.clone();
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0u8; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0, "fixture connection closed before request");
                    request.extend_from_slice(&buffer[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&request[..end]).unwrap();
                        let length = headers.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|length| length.trim().parse::<usize>().unwrap())).unwrap();
                        if request.len() >= end + 4 + length { break; }
                    }
                }
                #[derive(serde::Deserialize)]
                struct WireModel { model: String }
                let end = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
                let wire: WireModel = serde_json::from_slice(&request[end + 4..]).unwrap();
                observed_models.lock().unwrap().push(wire.model);
                let index = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let reply = replies.get(index).copied().unwrap_or(Reply::Limited);
                let (status, body) = match reply {
                    Reply::Limited => ("429 Too Many Requests", r#"{"type":"error","error":{"type":"rate_limit_error","message":"scripted limit"}}"#.to_string()),
                    Reply::Partial | Reply::Complete => {
                        let ending = if matches!(reply, Reply::Partial) {
                            "event: error\ndata: {\"error\":{\"type\":\"api_error\",\"message\":\"scripted partial failure\"}}\n\n"
                        } else {
                            "event: content_block_stop\ndata: {\"index\":0}\n\nevent: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {}\n\n"
                        };
                        ("200 OK", format!("event: message_start\ndata: {{\"message\":{{\"id\":\"local-{index}\",\"model\":\"claude-sonnet-4-6\",\"usage\":{{\"input_tokens\":7,\"output_tokens\":2}}}}}}\n\nevent: content_block_start\ndata: {{\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\nevent: content_block_delta\ndata: {{\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"Received answer {index}.\"}}}}\n\n{ending}"))
                    }
                };
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nRetry-After: 60\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }).await.expect("local HTTP fixture stalled");
        }
    });
    let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
        api_key: "fake-local-key".into(),
        api_base: format!("http://{address}"),
        max_retries: 0,
        request_timeout: std::time::Duration::from_secs(3),
        ..Default::default()
    })
    .unwrap();
    (Arc::new(client), requests, models, stop, server)
}

async fn dispatch(
    client: Arc<claurst_api::AnthropicClient>,
    context: ToolContext,
    messages: MessagesArc,
    tracker: Arc<CostTracker>,
) -> claurst_query::QueryOutcome {
    let (events, mut receiver) = tokio::sync::mpsc::channel(1);
    let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    let model = context
        .config
        .model
        .clone()
        .unwrap_or_else(|| "claude-sonnet-4-6".into());
    let mut handle = InteractiveQuery {
        client,
        context,
        tools: Arc::new(vec![]),
        config: claurst_query::QueryConfig {
            model,
            max_turns: 2,
            provider_registry: Some(Arc::new(claurst_api::ProviderRegistry::new())),
            model_registry: Some(Arc::new(claurst_api::ModelRegistry::new())),
            ..Default::default()
        },
        tracker,
        events,
        generation: claurst_tui::application::SessionGeneration::default(),
        cancel: tokio_util::sync::CancellationToken::new(),
    }
    .spawn(messages);
    let outcome = match tokio::time::timeout(std::time::Duration::from_secs(10), &mut handle).await
    {
        Ok(result) => result.unwrap(),
        Err(_) => {
            handle.abort();
            let _ = handle.await;
            drain.await.unwrap();
            panic!("query fixture timed out");
        }
    };
    // A finished turn has already delivered every event: the queue's only
    // sender lived in the worker, so the consumer sees it closed promptly.
    tokio::time::timeout(std::time::Duration::from_secs(1), drain)
        .await
        .expect("event queue must close once the turn has finished")
        .unwrap();
    outcome
}

#[test]
fn goal_dispatch_rate_limited_retry_does_not_adopt_replacement() {
    if isolated("goal_dispatch_rate_limited_retry_does_not_adopt_replacement") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
            for synthetic in [false, true] {
                for retry_kind in ["countdown", "manual", "model"] {
                    let mut session =
                        claurst_core::history::ConversationSession::new("claude-sonnet-4-6".into());
                    let path = claurst_core::GoalStore::default_path();
                    let store = claurst_core::GoalStore::open(&path).unwrap();
                    store.set_goal(&session.id, "original", None).unwrap();
                    let mut root = context(path);
                    root.session_id = session.id.clone();
                    let tracker = CostTracker::new();
                    tracker.add_usage(19, 0, 0, 0);
                    let mut first = root.clone();
                    let mut owner = InteractiveTurn::capture(&mut first, tracker.total_tokens());
                    let mut app = claurst_tui::App::new(Config::default(), tracker.clone());
                    let mut messages = vec![];
                    if synthetic {
                        owner
                            .inject(
                                "synthetic work".into(),
                                &mut messages,
                                &mut app,
                                &mut session,
                            )
                            .unwrap();
                    } else {
                        messages.push(claurst_core::types::Message::user("manual work"));
                    }
                    let before = messages.clone();
                    let history = Arc::new(tokio::sync::Mutex::new(messages));
                    let (client, requests, _models, stop, server) = rate_limit_server().await;
                    let outcome =
                        dispatch(client.clone(), first, history.clone(), tracker.clone()).await;
                    let claurst_query::QueryOutcome::Error(
                        claurst_core::error::ClaudeError::RateLimit {
                            retry_after_secs,
                            message,
                        },
                    ) = outcome
                    else {
                        panic!("expected rate limit: {outcome:?}");
                    };
                    app.open_rate_limit_recovery(message.unwrap_or_default(), retry_after_secs);
                    let replacement = store.set_goal(&session.id, "replacement", None).unwrap();
                    match retry_kind {
                        "countdown" => {
                            app.rate_limit_recovery.auto_retry_deadline =
                                Some(std::time::Instant::now());
                            app.rate_limit_recovery.tick();
                        }
                        "manual" => {
                            app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        }
                        "model" => {
                            app.handle_key_event(KeyEvent::new(
                                KeyCode::Char('h'),
                                KeyModifiers::NONE,
                            ));
                        }
                        _ => unreachable!(),
                    }
                    assert!(app.rate_limit_recovery.take_retry_directive().is_some());
                    let mut retry = root.clone();
                    retry.config = app.config.clone();
                    owner.prepare_context(&mut retry);
                    let outcome = dispatch(client, retry, history.clone(), tracker.clone()).await;
                    let _ = stop.send(());
                    server.await.unwrap();
                    assert!(matches!(outcome, claurst_query::QueryOutcome::Error(_)));
                    assert_eq!(
                        requests.load(std::sync::atomic::Ordering::SeqCst),
                        1,
                        "{retry_kind}"
                    );
                    assert_eq!(owner.baseline.as_ref().unwrap().tracker_total, 19);
                    assert_ne!(owner.baseline.as_ref().unwrap().goal_id, replacement.id);
                    let _ = owner.continuation(tracker.total_tokens());
                    assert_eq!(
                        store.try_get_goal(&session.id).unwrap().unwrap(),
                        replacement
                    );
                    let mut messages = history.lock().await.clone();
                    session.messages = messages.clone();
                    let mut commands = messages.clone();
                    assert!(!rollback_unattempted_turn(
                        Some(&owner),
                        false,
                        &mut messages,
                        &mut app,
                        &mut session,
                        &mut commands
                    ));
                    assert_eq!(
                        serde_json::to_string(&messages).unwrap(),
                        serde_json::to_string(&before).unwrap()
                    );
                    persist_interactive_session(&session, Some(&owner)).await;
                    assert_eq!(
                        serde_json::to_string(
                            &claurst_core::history::load_session(&session.id)
                                .await
                                .unwrap()
                                .messages
                        )
                        .unwrap(),
                        serde_json::to_string(&before).unwrap()
                    );
                }
            }
        });
}

#[test]
fn goal_dispatch_revoked_unattempted_uuid_is_absent_from_persistence() {
    if isolated("goal_dispatch_revoked_unattempted_uuid_is_absent_from_persistence") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut session =
                claurst_core::history::ConversationSession::new("claude-sonnet-4-6".into());
            let store = claurst_core::GoalStore::open_default().unwrap();
            store.set_goal(&session.id, "original", None).unwrap();
            let turn = store.capture_active_goal(&session.id).unwrap().unwrap();
            let mut owner = InteractiveTurn::from_goal(turn.clone(), 0);
            let mut ordinary = claurst_core::types::Message::user("ordinary retained");
            ordinary.uuid = Some("ordinary-control".into());
            let mut messages = vec![ordinary.clone()];
            let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
            app.replace_messages(messages.clone());
            owner
                .inject(
                    "synthetic unique searchable".into(),
                    &mut messages,
                    &mut app,
                    &mut session,
                )
                .unwrap();
            let owned = messages.last().unwrap().clone();
            let mut command_messages = messages.clone();
            claurst_core::history::save_session(&session).await.unwrap();
            assert_eq!(
                claurst_core::history::load_session(&session.id)
                    .await
                    .unwrap()
                    .messages
                    .len(),
                2
            );
            let replacement = store.set_goal(&session.id, "replacement", None).unwrap();
            assert!(turn.validate().is_err());
            assert!(!turn.request_attempted());
            assert!(rollback_unattempted_turn(
                Some(&owner),
                false,
                &mut messages,
                &mut app,
                &mut session,
                &mut command_messages
            ));
            assert!(
                !rollback_unattempted_turn(
                    Some(&owner),
                    false,
                    &mut messages,
                    &mut app,
                    &mut session,
                    &mut command_messages
                ),
                "idle frames must not rewrite an already rolled-back session"
            );
            persist_interactive_session(&session, Some(&owner)).await;
            assert_eq!(
                store.try_get_goal(&session.id).unwrap().unwrap(),
                replacement
            );
            let loaded = claurst_core::history::load_session(&session.id)
                .await
                .unwrap();
            assert!(
                !loaded.messages.iter().any(|m| m.uuid == owned.uuid),
                "unattempted owned UUID persisted after rollback"
            );
            assert!(loaded.messages.iter().any(|m| m.uuid == ordinary.uuid));
        });
}

fn command_context(
    session: &claurst_core::history::ConversationSession,
) -> claurst_commands::CommandContext {
    claurst_commands::CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: session.messages.clone(),
        working_dir: claurst_core::config::Settings::config_dir(),
        session_id: session.id.clone(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    }
}

#[derive(Clone, Copy, Debug)]
enum Revocation {
    Active,
    Replacement,
    Pause,
    Complete,
    Budget,
    Clear,
    StoreFailure,
}

fn revoke(
    store: &claurst_core::GoalStore,
    session: &str,
    case: Revocation,
    path: &std::path::Path,
) -> Option<claurst_core::Goal> {
    use claurst_core::GoalStatus;
    match case {
        Revocation::Active => {}
        Revocation::Replacement => {
            store.set_goal(session, "replacement", Some(999)).unwrap();
        }
        Revocation::Pause => store.set_status(session, GoalStatus::Paused).unwrap(),
        Revocation::Complete => store.set_status(session, GoalStatus::Complete).unwrap(),
        Revocation::Budget => store
            .set_status(session, GoalStatus::BudgetLimited)
            .unwrap(),
        Revocation::Clear => store.clear_goal(session).unwrap(),
        Revocation::StoreFailure => {
            std::fs::rename(path, path.with_extension("unavailable")).unwrap();
        }
    }
    store.try_get_goal(session).unwrap()
}

#[test]
fn goal_dispatch_payload_decision_injection_and_spawn_fences() {
    if isolated("goal_dispatch_payload_decision_injection_and_spawn_fences") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for payload in ["kickoff", "audit", "continuation"] {
                for before_injection in [true, false] {
                    for case in [
                        Revocation::Active,
                        Revocation::Replacement,
                        Revocation::Pause,
                        Revocation::Complete,
                        Revocation::Budget,
                        Revocation::Clear,
                        Revocation::StoreFailure,
                    ] {
                        // A unique app home binds the real command's default store; each
                        // captured turn must retain it after preparation selects another home.
                        let directory = tempfile::tempdir().unwrap();
                        std::env::set_var("COVEN_CODE_HOME", directory.path());
                        let mut session = claurst_core::history::ConversationSession::new(
                            "claude-sonnet-4-6".into(),
                        );
                        let mut command = command_context(&session);
                        let store = claurst_core::GoalStore::open_default().unwrap();
                        let path = claurst_core::GoalStore::default_path();
                        let (message, turn) = if payload == "continuation" {
                            let goal = store.set_goal(&session.id, "original", None).unwrap();
                            let claurst_query::GoalContinuation::Continue { message, turn } =
                                claurst_query::check_and_continue_goal_at_path_for_goal(
                                    &path,
                                    &session.id,
                                    &goal.id,
                                    5,
                                    2,
                                )
                                .unwrap()
                            else {
                                panic!("expected continuation");
                            };
                            (message, turn)
                        } else {
                            if payload == "audit" {
                                store.set_goal(&session.id, "original", None).unwrap();
                            }
                            let input = if payload == "audit" {
                                "/coven goal complete"
                            } else {
                                "/coven goal original"
                            };
                            let Some(claurst_commands::CommandResult::GoalUserMessage {
                                message,
                                turn,
                            }) = claurst_commands::execute_command(input, &mut command).await
                            else {
                                panic!("expected bound {payload}");
                            };
                            (message, turn)
                        };
                        let mut owner = InteractiveTurn::from_goal(turn, 11);
                        let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
                        let mut messages = vec![];
                        let expected =
                            before_injection.then(|| revoke(&store, &session.id, case, &path));
                        let injected = owner.inject(message, &mut messages, &mut app, &mut session);
                        if before_injection && !matches!(case, Revocation::Active) {
                            assert!(injected.is_err(), "{payload}: {case:?}");
                            assert!(
                                messages.is_empty()
                                    && app.messages.is_empty()
                                    && session.messages.is_empty()
                            );
                            assert_eq!(store.try_get_goal(&session.id).unwrap(), expected.unwrap());
                            continue;
                        }
                        injected.unwrap();
                        let expected =
                            expected.unwrap_or_else(|| revoke(&store, &session.id, case, &path));
                        let (client, requests, _models, stop, server) = rate_limit_server().await;
                        let mut ctx = context(path.clone());
                        ctx.session_id = session.id.clone();
                        owner.prepare_context(&mut ctx);
                        // The common actual spawned task checks after all caller preparation.
                        let history = Arc::new(tokio::sync::Mutex::new(messages.clone()));
                        let outcome =
                            dispatch(client, ctx, history.clone(), CostTracker::new()).await;
                        let _ = stop.send(());
                        server.await.unwrap();
                        assert!(matches!(outcome, claurst_query::QueryOutcome::Error(_)));
                        assert_eq!(
                            requests.load(std::sync::atomic::Ordering::SeqCst),
                            usize::from(matches!(case, Revocation::Active)),
                            "{payload}: {case:?}"
                        );
                        assert_eq!(
                            store.try_get_goal(&session.id).unwrap(),
                            expected,
                            "replacement/counters changed"
                        );
                        messages = history.lock().await.clone();
                        rollback_unattempted_turn(
                            Some(&owner),
                            false,
                            &mut messages,
                            &mut app,
                            &mut session,
                            &mut command.messages,
                        );
                        assert_eq!(messages.is_empty(), !matches!(case, Revocation::Active));
                    }
                }
            }
        });
}

#[test]
fn goal_dispatch_resolved_absence_failure_and_original_store_survive_retry() {
    if isolated("goal_dispatch_resolved_absence_failure_and_original_store_survive_retry") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for failed in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("goals.sqlite");
                if failed {
                    std::fs::create_dir(&path).unwrap();
                }
                let mut initial = context(path.clone());
                let owner = InteractiveTurn::capture(&mut initial, 17);
                if failed {
                    std::fs::remove_dir(&path).unwrap();
                }
                let store = claurst_core::GoalStore::open(&path).unwrap();
                let replacement = store.set_goal("dispatch", "newly active", None).unwrap();
                for _ in 0..2 {
                    let mut retry = context(path.clone());
                    owner.prepare_context(&mut retry);
                    assert!(
                        matches!(
                            &retry.goal_turn,
                            claurst_core::goal::GoalTurnContext::Failed(_)
                        ) == failed
                    );
                    assert!(retry.goal_turn.captured().is_none());
                    let (client, requests, _models, stop, server) = rate_limit_server().await;
                    let _ = dispatch(
                        client,
                        retry,
                        Arc::new(tokio::sync::Mutex::new(vec![
                            claurst_core::types::Message::user("ordinary"),
                        ])),
                        CostTracker::new(),
                    )
                    .await;
                    let _ = stop.send(());
                    server.await.unwrap();
                    assert_eq!(
                        requests.load(std::sync::atomic::Ordering::SeqCst),
                        usize::from(!failed)
                    );
                    assert!(matches!(
                        owner.continuation(30),
                        claurst_query::GoalContinuation::NoGoal
                    ));
                    assert_eq!(
                        store.try_get_goal("dispatch").unwrap().unwrap(),
                        replacement
                    );
                }
            }
            let original = tempfile::tempdir().unwrap();
            let other = tempfile::tempdir().unwrap();
            let path = original.path().join("goals.sqlite");
            let store = claurst_core::GoalStore::open(&path).unwrap();
            store.set_goal("dispatch", "original", None).unwrap();
            let mut ctx = context(path.clone());
            let owner = InteractiveTurn::capture(&mut ctx, 0);
            let other_path = other.path().join("goals.sqlite");
            std::fs::copy(&path, &other_path).unwrap();
            std::env::set_var("COVEN_CODE_HOME", other.path());
            store
                .set_status("dispatch", claurst_core::GoalStatus::Paused)
                .unwrap();
            let copied = claurst_core::GoalStore::open(&other_path).unwrap();
            let copied_before = copied.try_get_goal("dispatch").unwrap();
            let mut retry = context(other_path);
            owner.prepare_context(&mut retry);
            assert_eq!(
                retry.goal_turn.captured().unwrap().store_path(),
                path.canonicalize().unwrap()
            );
            let (client, requests, _models, stop, server) = rate_limit_server().await;
            let _ = dispatch(
                client,
                retry,
                Arc::new(tokio::sync::Mutex::new(vec![
                    claurst_core::types::Message::user("bound"),
                ])),
                CostTracker::new(),
            )
            .await;
            let _ = stop.send(());
            server.await.unwrap();
            assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert_eq!(copied.try_get_goal("dispatch").unwrap(), copied_before);
        });
}

#[test]
fn goal_dispatch_preparation_failure_then_revoke_rolls_back_json_and_index() {
    if isolated("goal_dispatch_preparation_failure_then_revoke_rolls_back_json_and_index") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for (equal_text, recover) in [(false, false), (true, false), (false, true)] {
                let mut session =
                    claurst_core::history::ConversationSession::new("claude-sonnet-4-6".into());
                let store = claurst_core::GoalStore::open_default().unwrap();
                store.set_goal(&session.id, "original", None).unwrap();
                let turn = store.capture_active_goal(&session.id).unwrap().unwrap();
                let mut owner = InteractiveTurn::from_goal(turn.clone(), 0);
                let text = format!("synthetic-{}", session.id);
                let mut ordinary = claurst_core::types::Message::user(if equal_text {
                    text.clone()
                } else {
                    "ordinary control".into()
                });
                ordinary.uuid = Some(uuid::Uuid::new_v4().to_string());
                let mut messages = vec![ordinary.clone()];
                let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
                app.replace_messages(messages.clone());
                owner
                    .inject(text.clone(), &mut messages, &mut app, &mut session)
                    .unwrap();
                let owned = owner.synthetic_uuid.clone().unwrap();
                claurst_core::history::create_checkpoint(&mut session, Some("early metadata"));
                let mut command_messages = messages.clone();
                claurst_core::history::save_session(&session).await.unwrap();
                assert!(claurst_core::history::load_session(&session.id)
                    .await
                    .unwrap()
                    .messages
                    .iter()
                    .any(|m| m.uuid.as_deref() == Some(&owned)));
                let (client, requests, _models, stop, server) =
                    scripted_server(vec![Reply::Complete]).await;
                let mut ctx = context(claurst_core::GoalStore::default_path());
                ctx.session_id = session.id.clone();
                ctx.config.provider = Some("codex".into()); // Empty auth: fails before a model transport.
                ctx.config.model = Some("gpt-5.4".into());
                owner.prepare_context(&mut ctx);
                let history = Arc::new(tokio::sync::Mutex::new(messages.clone()));
                let outcome =
                    dispatch(client.clone(), ctx, history.clone(), CostTracker::new()).await;
                let claurst_query::QueryOutcome::Error(error) = outcome else {
                    panic!("expected auth failure: {outcome:?}");
                };
                assert!(
                    error
                        .to_string()
                        .contains("No API key for provider 'codex'"),
                    "{error}"
                );
                assert!(!turn.request_attempted());
                messages = history.lock().await.clone();
                session.messages = messages.clone();
                persist_interactive_session(&session, Some(&owner)).await;
                let index = claurst_core::SqliteSessionStore::open(
                    &Settings::config_dir().join("sessions.db"),
                )
                .unwrap();
                assert_eq!(
                    index
                        .list_sessions()
                        .unwrap()
                        .iter()
                        .find(|s| s.id == session.id)
                        .unwrap()
                        .message_count,
                    1
                );
                assert_eq!(
                    index
                        .search_sessions(&text)
                        .unwrap()
                        .iter()
                        .any(|s| s.id == session.id),
                    equal_text
                );
                if recover {
                    let mut retry = context(claurst_core::GoalStore::default_path());
                    retry.session_id = session.id.clone();
                    owner.prepare_context(&mut retry);
                    let outcome =
                        dispatch(client, retry, history.clone(), CostTracker::new()).await;
                    let _ = stop.send(());
                    server.await.unwrap();
                    assert!(matches!(
                        outcome,
                        claurst_query::QueryOutcome::EndTurn { .. }
                    ));
                    assert!(turn.request_attempted());
                    assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
                    session.messages = history.lock().await.clone();
                    persist_interactive_session(&session, Some(&owner)).await;
                    assert_eq!(
                        index
                            .list_sessions()
                            .unwrap()
                            .iter()
                            .find(|s| s.id == session.id)
                            .unwrap()
                            .message_count,
                        3
                    );
                    assert!(index
                        .search_sessions(&text)
                        .unwrap()
                        .iter()
                        .any(|s| s.id == session.id));
                    continue;
                }
                let replacement = store.set_goal(&session.id, "replacement", None).unwrap();
                let mut retry = context(claurst_core::GoalStore::default_path());
                retry.session_id = session.id.clone();
                owner.prepare_context(&mut retry);
                let _ = dispatch(client, retry, history.clone(), CostTracker::new()).await;
                let _ = stop.send(());
                server.await.unwrap();
                assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 0);
                messages = history.lock().await.clone();
                session.messages = messages.clone();
                assert!(rollback_unattempted_turn(
                    Some(&owner),
                    false,
                    &mut messages,
                    &mut app,
                    &mut session,
                    &mut command_messages
                ));
                *history.lock().await = messages.clone();
                persist_interactive_session(&session, Some(&owner)).await;
                let loaded = claurst_core::history::load_session(&session.id)
                    .await
                    .unwrap();
                for copy in [
                    &messages,
                    &app.messages,
                    &command_messages,
                    &session.messages,
                    &loaded.messages,
                    &loaded.checkpoints[0].snapshot,
                ] {
                    assert!(!copy.iter().any(|m| m.uuid.as_deref() == Some(&owned)));
                    assert!(copy.iter().any(|m| m.uuid == ordinary.uuid));
                }
                assert_eq!(
                    store.try_get_goal(&session.id).unwrap().unwrap(),
                    replacement
                );
                assert_eq!(
                    index
                        .list_sessions()
                        .unwrap()
                        .iter()
                        .find(|s| s.id == session.id)
                        .unwrap()
                        .message_count,
                    1
                );
            }
        });
}

#[test]
fn goal_dispatch_repeated_partial_failures_keep_manual_and_synthetic_baselines() {
    if isolated("goal_dispatch_repeated_partial_failures_keep_manual_and_synthetic_baselines") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for (synthetic, replace) in [(false, false), (true, false), (false, true), (true, true)]
            {
                let mut session =
                    claurst_core::history::ConversationSession::new("claude-sonnet-4-6".into());
                let store = claurst_core::GoalStore::open_default().unwrap();
                store.set_goal(&session.id, "original", None).unwrap();
                let tracker = CostTracker::new();
                tracker.add_usage(100, 0, 0, 0);
                let mut root = context(claurst_core::GoalStore::default_path());
                root.session_id = session.id.clone();
                let mut captured = root.clone();
                let mut owner = InteractiveTurn::capture(&mut captured, tracker.total_tokens());
                owner.baseline.as_mut().unwrap().started_at -= std::time::Duration::from_secs(3);
                let started = owner.baseline.as_ref().unwrap().started_at;
                let mut app = claurst_tui::App::new(Config::default(), tracker.clone());
                let mut messages = vec![];
                if synthetic {
                    owner
                        .inject(
                            "synthetic work".into(),
                            &mut messages,
                            &mut app,
                            &mut session,
                        )
                        .unwrap();
                } else {
                    messages.push(claurst_core::types::Message::user("manual work"));
                }
                let history = Arc::new(tokio::sync::Mutex::new(messages));
                let (client, requests, models, stop, server) =
                    scripted_server(vec![Reply::Partial, Reply::Partial, Reply::Complete]).await;
                let mut replacement = None;
                for attempt in 0..3 {
                    if replace && attempt == 2 {
                        replacement =
                            Some(store.set_goal(&session.id, "replacement", None).unwrap());
                    }
                    let mut ctx = root.clone();
                    owner.prepare_context(&mut ctx);
                    if attempt > 0 {
                        app.set_model("claude-haiku-4-5-20251001".into());
                        ctx.config.model = app.config.model.clone();
                    }
                    let outcome =
                        dispatch(client.clone(), ctx, history.clone(), tracker.clone()).await;
                    assert_eq!(
                        matches!(outcome, claurst_query::QueryOutcome::EndTurn { .. }),
                        attempt == 2 && !replace
                    );
                    assert_eq!(owner.baseline.as_ref().unwrap().tracker_total, 100);
                    assert_eq!(owner.baseline.as_ref().unwrap().started_at, started);
                }
                let _ = stop.send(());
                server.await.unwrap();
                let attempts = if replace { 2 } else { 3 };
                assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), attempts);
                assert_eq!(tracker.total_tokens(), 100 + 9 * attempts as u64);
                assert_eq!(
                    *models.lock().unwrap(),
                    [
                        "claude-sonnet-4-6",
                        "claude-haiku-4-5-20251001",
                        "claude-haiku-4-5-20251001"
                    ][..attempts]
                );
                let messages = history.lock().await.clone();
                assert_eq!(
                    messages
                        .iter()
                        .filter(|m| m.role == claurst_core::types::Role::Assistant)
                        .count(),
                    attempts
                );
                let uuids: std::collections::HashSet<_> = messages
                    .iter()
                    .filter(|m| m.role == claurst_core::types::Role::Assistant)
                    .map(|m| m.uuid.clone().unwrap())
                    .collect();
                assert_eq!(uuids.len(), attempts);
                let continuation = owner.continuation(tracker.total_tokens());
                if let Some(replacement) = replacement {
                    assert!(matches!(
                        continuation,
                        claurst_query::GoalContinuation::Stop { .. }
                    ));
                    assert_eq!(
                        store.try_get_goal(&session.id).unwrap().unwrap(),
                        replacement
                    );
                    let mut retained = messages.clone();
                    session.messages = messages.clone();
                    let mut commands = messages.clone();
                    assert!(!rollback_unattempted_turn(
                        Some(&owner),
                        false,
                        &mut retained,
                        &mut app,
                        &mut session,
                        &mut commands
                    ));
                    assert_eq!(
                        serde_json::to_string(&retained).unwrap(),
                        serde_json::to_string(&messages).unwrap()
                    );
                } else {
                    assert!(matches!(
                        continuation,
                        claurst_query::GoalContinuation::Continue { .. }
                    ));
                    let goal = store.try_get_goal(&session.id).unwrap().unwrap();
                    assert_eq!(goal.tokens_used, 27);
                    assert_eq!(goal.turns_used, 1);
                    assert!(goal.time_used_secs >= 3);
                }
                session.messages = messages;
                persist_interactive_session(&session, Some(&owner)).await;
                if synthetic {
                    let index = claurst_core::SqliteSessionStore::open(
                        &Settings::config_dir().join("sessions.db"),
                    )
                    .unwrap();
                    assert!(index
                        .search_sessions("synthetic work")
                        .unwrap()
                        .iter()
                        .any(|s| s.id == session.id));
                }
            }
        });
}

#[test]
fn goal_dispatch_abandonment_clears_pending_retry_and_owned_local_work() {
    if isolated("goal_dispatch_abandonment_clears_pending_retry_and_owned_local_work") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            use claurst_core::keybindings::{KeybindingResolver, UserBinding, UserKeybindings};
            use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
            for action in ["dismiss", "configured", "remote"] {
                for pending in [false, true] {
                    if action == "dismiss" && pending {
                        continue;
                    } // No modal remains to dismiss.
                    for attempted in [false, true] {
                        let mut session = claurst_core::history::ConversationSession::new(
                            "claude-sonnet-4-6".into(),
                        );
                        let store = claurst_core::GoalStore::open_default().unwrap();
                        store.set_goal(&session.id, "original", None).unwrap();
                        let turn = store.capture_active_goal(&session.id).unwrap().unwrap();
                        let mut submitted = InteractiveTurn::from_goal(turn.clone(), 10);
                        let mut ordinary = claurst_core::types::Message::user("ordinary retained");
                        ordinary.uuid = Some(uuid::Uuid::new_v4().to_string());
                        let mut messages = vec![ordinary.clone()];
                        let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
                        app.replace_messages(messages.clone());
                        submitted
                            .inject(
                                "synthetic pending".into(),
                                &mut messages,
                                &mut app,
                                &mut session,
                            )
                            .unwrap();
                        let owned = submitted.synthetic_uuid.clone().unwrap();
                        if attempted {
                            turn.mark_request_attempted();
                            let partial = claurst_core::types::Message::assistant(
                                "received partial response",
                            );
                            messages.push(partial.clone());
                            app.push_message(partial);
                            session.messages = messages.clone();
                        }
                        let mut command_messages = messages.clone();
                        claurst_core::history::save_session(&session).await.unwrap();
                        let mut owner = Some(submitted);
                        let cancel = tokio_util::sync::CancellationToken::new();
                        app.open_rate_limit_recovery("limited".into(), Some(60));
                        if pending {
                            app.rate_limit_recovery.request_retry(None);
                        }
                        match action {
                            "dismiss" => {
                                app.handle_key_event(KeyEvent::new(
                                    KeyCode::Esc,
                                    KeyModifiers::NONE,
                                ));
                            }
                            "configured" => {
                                app.keybindings = KeybindingResolver::new(&UserKeybindings {
                                    bindings: vec![UserBinding {
                                        chord: "alt+x".into(),
                                        action: Some("interrupt".into()),
                                        context: None,
                                    }],
                                    ..Default::default()
                                });
                                assert!(handle_exit_key(
                                    &mut app,
                                    KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
                                    &Some(cancel.clone()),
                                    &mut owner
                                ));
                                assert!(cancel.is_cancelled());
                            }
                            "remote" => {
                                abandon_interactive_turn(
                                    &mut owner,
                                    &mut app,
                                    &Some(cancel.clone()),
                                );
                                assert!(cancel.is_cancelled());
                            }
                            _ => unreachable!(),
                        }
                        assert!(!app.rate_limit_recovery.is_pending());
                        assert!(app.rate_limit_recovery.auto_retry_deadline.is_none());
                        assert!(app.rate_limit_recovery.take_retry_directive().is_none());
                        // Use the production frame seam, including its actual JSON write.
                        settle_abandoned_turn(
                            &mut owner,
                            false,
                            &mut messages,
                            &mut app,
                            &mut session,
                            &mut command_messages,
                        )
                        .await;
                        assert!(owner.is_none());
                        let loaded = claurst_core::history::load_session(&session.id)
                            .await
                            .unwrap();
                        for copy in [
                            &messages,
                            &app.messages,
                            &session.messages,
                            &command_messages,
                            &loaded.messages,
                        ] {
                            assert_eq!(
                                copy.iter().any(|m| m.uuid.as_deref() == Some(&owned)),
                                attempted,
                                "{action}, pending={pending}"
                            );
                            assert!(copy.iter().any(|m| m.uuid == ordinary.uuid));
                            assert_eq!(
                                copy.iter()
                                    .any(|m| m.get_all_text() == "received partial response"),
                                attempted
                            );
                        }
                        let replacement = store
                            .set_goal(&session.id, "new independent request", None)
                            .unwrap();
                        let mut next = context(claurst_core::GoalStore::default_path());
                        next.session_id = session.id.clone();
                        let independent = InteractiveTurn::capture(&mut next, 20);
                        assert_eq!(
                            independent.baseline.as_ref().unwrap().goal_id,
                            replacement.id
                        );
                        let (client, requests, _models, stop, server) = rate_limit_server().await;
                        let _ = dispatch(
                            client,
                            next,
                            Arc::new(tokio::sync::Mutex::new(messages)),
                            CostTracker::new(),
                        )
                        .await;
                        let _ = stop.send(());
                        server.await.unwrap();
                        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
                        assert_eq!(
                            store.try_get_goal(&session.id).unwrap().unwrap(),
                            replacement
                        );
                    }
                }
            }
        });
}

#[test]
fn goal_dispatch_successive_continuations_own_independent_attempt_state() {
    if isolated("goal_dispatch_successive_continuations_own_independent_attempt_state") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut session =
                claurst_core::history::ConversationSession::new("claude-sonnet-4-6".into());
            let path = claurst_core::GoalStore::default_path();
            let store = claurst_core::GoalStore::open(&path).unwrap();
            let goal = store.set_goal(&session.id, "original", None).unwrap();
            let mut messages = vec![];
            let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
            let claurst_query::GoalContinuation::Continue { message, turn } =
                claurst_query::check_and_continue_goal_at_path_for_goal(
                    &path,
                    &session.id,
                    &goal.id,
                    0,
                    0,
                )
                .unwrap()
            else {
                panic!("first continuation");
            };
            let mut first = InteractiveTurn::from_goal(turn, 0);
            first
                .inject(message, &mut messages, &mut app, &mut session)
                .unwrap();
            let old_child = first.goal.for_child_work();
            let housekeeping = first.goal.for_background_work();
            // A housekeeping transport carries this query's transcript and must retain it.
            housekeeping.captured().unwrap().mark_request_attempted();
            assert!(first.goal.captured().unwrap().request_attempted());
            let claurst_query::GoalContinuation::Continue { message, turn } =
                claurst_query::check_and_continue_goal_at_path_for_goal(
                    &path,
                    &session.id,
                    &goal.id,
                    0,
                    0,
                )
                .unwrap()
            else {
                panic!("next continuation");
            };
            let mut next = InteractiveTurn::from_goal(turn, 0);
            next.inject(message, &mut messages, &mut app, &mut session)
                .unwrap();
            assert_ne!(first.synthetic_uuid, next.synthetic_uuid);
            old_child.captured().unwrap().mark_request_attempted();
            assert!(!next.goal.captured().unwrap().request_attempted());
            store
                .set_status(&session.id, claurst_core::GoalStatus::Paused)
                .unwrap();
            let mut command_messages = messages.clone();
            assert!(!rollback_unattempted_turn(
                Some(&first),
                false,
                &mut messages,
                &mut app,
                &mut session,
                &mut command_messages
            ));
            assert!(rollback_unattempted_turn(
                Some(&next),
                false,
                &mut messages,
                &mut app,
                &mut session,
                &mut command_messages
            ));
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].uuid, first.synthetic_uuid);
        });
}

#[test]
fn goal_dispatch_abandoned_inflight_outcome_does_not_rearm_recovery() {
    if isolated("goal_dispatch_abandoned_inflight_outcome_does_not_rearm_recovery") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut session =
                claurst_core::history::ConversationSession::new("claude-sonnet-4-6".into());
            let store = claurst_core::GoalStore::open_default().unwrap();
            store.set_goal(&session.id, "original", None).unwrap();
            let mut ctx = context(claurst_core::GoalStore::default_path());
            ctx.session_id = session.id.clone();
            let mut owner = Some(InteractiveTurn::capture(&mut ctx, 0));
            let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
            let mut messages = vec![];
            owner
                .as_mut()
                .unwrap()
                .inject(
                    "attempted synthetic".into(),
                    &mut messages,
                    &mut app,
                    &mut session,
                )
                .unwrap();
            let history = Arc::new(tokio::sync::Mutex::new(messages.clone()));
            let (client, requests, _models, stop, server) = rate_limit_server().await;
            // The task has finished, but the REPL has not yet handled its outcome.
            let outcome = dispatch(client, ctx, history.clone(), CostTracker::new()).await;
            let _ = stop.send(());
            server.await.unwrap();
            assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
            abandon_interactive_turn(&mut owner, &mut app, &None);
            let mut commands = messages.clone();
            settle_abandoned_turn(
                &mut owner,
                true,
                &mut messages,
                &mut app,
                &mut session,
                &mut commands,
            )
            .await;
            assert!(owner.as_ref().unwrap().abandoned);
            let claurst_query::QueryOutcome::Error(claurst_core::error::ClaudeError::RateLimit {
                message,
                retry_after_secs,
            }) = outcome
            else {
                panic!("expected 429");
            };
            show_rate_limit_recovery(
                owner.as_ref(),
                &mut app,
                message.as_deref(),
                retry_after_secs,
            );
            assert!(!app.rate_limit_recovery.is_pending());
            assert!(app.rate_limit_recovery.auto_retry_deadline.is_none());
            messages = history.lock().await.clone();
            session.messages = messages.clone();
            persist_interactive_session(&session, owner.as_ref()).await;
            settle_abandoned_turn(
                &mut owner,
                false,
                &mut messages,
                &mut app,
                &mut session,
                &mut commands,
            )
            .await;
            assert!(owner.is_none());
            assert_eq!(messages.len(), 1);
            assert_eq!(
                claurst_core::history::load_session(&session.id)
                    .await
                    .unwrap()
                    .messages[0]
                    .get_all_text(),
                "attempted synthetic"
            );
        });
}

#[test]
fn goal_dispatch_file_preparation_blocks_idle_compaction() {
    if isolated("goal_dispatch_file_preparation_blocks_idle_compaction") {
        return;
    }
    let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
    app.auto_compact_enabled = true;
    app.context_window_size = 100;
    app.context_used_tokens = 99;
    app.auto_compact_threshold = 90;
    app.file_injection_dialog
        .show("@large.txt".into(), vec![], vec![], 1, None);
    assert_eq!(
        select_idle_compaction(&app, false),
        None,
        "file preparation must retain its original owner"
    );
    app.file_injection_dialog.dismiss();
    app.file_injection_force = true;
    assert_eq!(
        select_idle_compaction(&app, false),
        None,
        "prepared resubmission must retain its owner"
    );
}

#[test]
fn goal_dispatch_file_confirmation_retains_original_capture_and_abort_releases_it() {
    if isolated("goal_dispatch_file_confirmation_retains_original_capture_and_abort_releases_it") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
            for initial in ["goal", "none", "failed"] {
                for action in ["confirm", "abort", "cancel"] {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("goals.sqlite");
                    if initial == "failed" {
                        std::fs::create_dir(&path).unwrap();
                    }
                    if initial == "goal" {
                        claurst_core::GoalStore::open(&path)
                            .unwrap()
                            .set_goal("dispatch", "original", None)
                            .unwrap();
                    }
                    let root = context(path.clone());
                    let mut owner =
                        Some(select_submission_turn(None, &mut None, &root, 100, false).unwrap());
                    let baseline = owner.as_ref().unwrap().baseline.clone();
                    owner.as_mut().unwrap().awaiting_file_confirmation = true;
                    let mut app = claurst_tui::App::new(Config::default(), CostTracker::new());
                    let file = dir.path().join("large.txt");
                    std::fs::write(&file, "x".repeat(2048)).unwrap();
                    let input = "inspect @large.txt";
                    let (_, refs) =
                        claurst_tui::file_injection::parse_at_refs(input, dir.path(), 1);
                    assert_eq!(refs.len(), 1);
                    app.file_injection_dialog.show(
                        input.into(),
                        vec![],
                        refs.into_iter()
                            .map(|r| (r.path.display().to_string(), r.size_kb, r.issue.unwrap()))
                            .collect(),
                        1,
                        Some(dir.path().into()),
                    );
                    if initial == "failed" {
                        std::fs::remove_dir(&path).unwrap();
                    }
                    let store = claurst_core::GoalStore::open(&path).unwrap();
                    let replacement = store.set_goal("dispatch", "replacement", None).unwrap();
                    if action == "cancel" {
                        abandon_interactive_turn(&mut owner, &mut app, &None);
                    } else {
                        app.handle_key_event(KeyEvent::new(
                            if action == "confirm" {
                                KeyCode::Enter
                            } else {
                                KeyCode::Esc
                            },
                            KeyModifiers::NONE,
                        ));
                    }
                    handle_file_preparation(&mut app, &mut owner);
                    assert_eq!(app.file_injection_force, action == "confirm");
                    if action != "confirm" {
                        assert!(owner.is_none());
                    }
                    let selected = select_submission_turn(
                        None,
                        &mut owner,
                        &root,
                        200,
                        app.file_injection_force,
                    )
                    .unwrap();
                    if action == "confirm" {
                        assert_eq!(
                            selected.baseline.as_ref().map(|b| (
                                b.goal_id.as_str(),
                                b.tracker_total,
                                b.started_at
                            )),
                            baseline.as_ref().map(|b| (
                                b.goal_id.as_str(),
                                b.tracker_total,
                                b.started_at
                            ))
                        );
                        assert_eq!(
                            matches!(
                                selected.goal,
                                claurst_core::goal::GoalTurnContext::Failed(_)
                            ),
                            initial == "failed"
                        );
                    } else {
                        assert_eq!(selected.baseline.as_ref().unwrap().goal_id, replacement.id);
                    }
                    let mut ctx = root.clone();
                    selected.prepare_context(&mut ctx);
                    let (client, requests, _models, stop, server) = rate_limit_server().await;
                    let _ = dispatch(
                        client,
                        ctx,
                        Arc::new(tokio::sync::Mutex::new(vec![
                            claurst_core::types::Message::user(input),
                        ])),
                        CostTracker::new(),
                    )
                    .await;
                    let _ = stop.send(());
                    server.await.unwrap();
                    assert_eq!(
                        requests.load(std::sync::atomic::Ordering::SeqCst),
                        usize::from(action != "confirm" || initial == "none"),
                        "{initial}/{action}"
                    );
                    assert_eq!(
                        store.try_get_goal("dispatch").unwrap().unwrap(),
                        replacement
                    );
                }
            }
        });
}
