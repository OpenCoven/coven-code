use super::*;
use claurst_core::plugin_mcp::{PluginMcpContribution, PluginMcpOrigin, PluginMcpSource};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct CountingBackend(
    AtomicUsize,
    std::sync::Mutex<Option<BoxStream<'static, anyhow::Result<Value>>>>,
);

#[async_trait]
impl backend::McpClientBackend for CountingBackend {
    fn kind(&self) -> backend::McpBackendKind {
        backend::McpBackendKind::Rmcp
    }
    fn snapshot(&self) -> backend::McpClientSnapshot {
        backend::McpClientSnapshot::empty("plugin")
    }
    async fn list_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    async fn call_tool(&self, _: &str, _: Option<Value>) -> anyhow::Result<CallToolResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(CallToolResult {
            content: vec![],
            is_error: false,
        })
    }
    async fn list_resources(&self) -> anyhow::Result<Vec<McpResource>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    async fn read_resource(&self, uri: &str) -> anyhow::Result<ResourceContents> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ResourceContents {
            uri: uri.into(),
            mime_type: None,
            text: None,
            blob: None,
        })
    }
    async fn list_prompts(&self) -> anyhow::Result<Vec<McpPrompt>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    async fn get_prompt(
        &self,
        _: &str,
        _: Option<HashMap<String, String>>,
    ) -> anyhow::Result<GetPromptResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(GetPromptResult {
            description: None,
            messages: vec![],
        })
    }
    async fn subscribe_resource(&self, _: &str) -> anyhow::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn unsubscribe_resource(&self, _: &str) -> anyhow::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn subscribe_to_notifications(&self) -> BoxStream<'static, anyhow::Result<Value>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        self.1
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Box::pin(futures::stream::pending()))
    }
    fn shutdown_notifications(&self) {}
    async fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

fn contribution(project: &Path, source: PluginMcpSource) -> PluginMcpContribution {
    PluginMcpContribution::new(
        PluginMcpOrigin::new(source, project, Some(project)).unwrap(),
        McpServerConfig {
            name: "plugin".into(),
            command: Some("unused".into()),
            args: vec![],
            env: Default::default(),
            url: None,
            server_type: "stdio".into(),
        },
    )
}

async fn client_boundary_results(client: &McpClient, caller: Option<&Path>) -> Vec<bool> {
    vec![
        client.list_tools(caller).await.is_ok(),
        client.call_tool(caller, "tool", None).await.is_ok(),
        client.list_resources(caller).await.is_ok(),
        client
            .read_resource(caller, "test://resource")
            .await
            .is_ok(),
        client.list_prompts(caller).await.is_ok(),
        client.get_prompt(caller, "prompt", None).await.is_ok(),
        client
            .subscribe_resource(caller, "test://resource")
            .await
            .is_ok(),
        client
            .unsubscribe_resource(caller, "test://resource")
            .await
            .is_ok(),
        client.subscribe_to_notifications(caller).is_ok(),
    ]
}

#[tokio::test]
async fn all_client_boundaries_require_actual_canonical_project_and_live_authority() {
    let project = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    let plugin = contribution(project.path(), PluginMcpSource::Project);
    let backend = Arc::new(CountingBackend::default());
    let client = McpClient::from_test_plugin_backend(backend.clone(), plugin.clone());
    for caller in [
        None,
        Some(foreign.path()),
        Some(project.path().join("missing").as_path()),
    ] {
        assert_eq!(
            client_boundary_results(&client, caller).await,
            vec![false; 9]
        );
        assert_eq!(backend.0.load(Ordering::SeqCst), 0);
    }
    assert_eq!(
        client_boundary_results(&client, Some(&project.path().join("."))).await,
        vec![true; 9]
    );
    assert_eq!(backend.0.load(Ordering::SeqCst), 9);
    #[cfg(unix)]
    {
        let aliases = tempfile::tempdir().unwrap();
        let alias = aliases.path().join("alias");
        std::os::unix::fs::symlink(project.path(), &alias).unwrap();
        assert_eq!(
            client_boundary_results(&client, Some(&alias)).await,
            vec![true; 9]
        );
    }
    let before_revoke = backend.0.load(Ordering::SeqCst);
    plugin.revoke();
    assert_eq!(
        client_boundary_results(&client, Some(project.path())).await,
        vec![false; 9]
    );
    assert_eq!(backend.0.load(Ordering::SeqCst), before_revoke);
}

#[tokio::test]
async fn manager_and_free_functions_cannot_bypass_client_authority() {
    let project = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    let plugin = contribution(project.path(), PluginMcpSource::Project);
    let backend = Arc::new(CountingBackend::default());
    let mut manager = McpManager::new();
    manager.clients.insert(
        "plugin".into(),
        Arc::new(McpClient::from_test_plugin_backend(
            backend.clone(),
            plugin.clone(),
        )),
    );
    manager
        .plugin_authorities
        .insert("plugin".into(), plugin.clone());
    manager
        .server_configs
        .insert("plugin".into(), plugin.config.clone());
    for caller in [None, Some(foreign.path())] {
        assert!(manager
            .call_tool(caller, "plugin_tool", None)
            .await
            .is_err());
        assert!(manager.list_all_resources(caller, None).await.is_empty());
        assert!(manager
            .read_resource(caller, "plugin", "uri")
            .await
            .is_err());
        assert!(manager.list_all_prompts(caller, None).await.is_empty());
        assert!(manager
            .get_prompt(caller, "plugin", "prompt", None)
            .await
            .is_err());
        assert!(subscribe_resource(&manager, caller, "plugin", "uri")
            .await
            .recv()
            .await
            .is_none());
        assert!(unsubscribe_resource(&manager, caller, "plugin", "uri")
            .await
            .is_err());
        assert!(matches!(
            manager.auth_state(caller, "plugin"),
            McpAuthState::Error(_)
        ));
        for result in [
            manager.initiate_auth(caller, "plugin").await.map(|_| ()),
            manager.begin_auth(caller, "plugin").await.map(|_| ()),
            manager.authenticate(caller, "plugin").await.map(|_| ()),
            manager.store_token(caller, "plugin", "fixture", None),
        ] {
            assert!(result.unwrap_err().to_string().contains("caller project"));
        }
        assert!(manager.load_token(caller, "plugin").await.is_none());
        assert_eq!(backend.0.load(Ordering::SeqCst), 0);
    }
    plugin.revoke();
    assert!(manager
        .call_tool(Some(project.path()), "plugin_tool", None)
        .await
        .is_err());
    assert_eq!(backend.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn global_plugin_and_settings_clients_keep_cross_project_access() {
    let project = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    let plugin = contribution(project.path(), PluginMcpSource::User);
    let backend = Arc::new(CountingBackend::default());
    let client = McpClient::from_test_plugin_backend(backend.clone(), plugin);
    assert_eq!(
        client_boundary_results(&client, Some(foreign.path())).await,
        vec![true; 9]
    );
    let user = McpClient::from_test_backend(backend.clone());
    assert_eq!(client_boundary_results(&user, None).await, vec![true; 9]);
    assert_eq!(backend.0.load(Ordering::SeqCst), 18);
}

#[tokio::test]
async fn revoked_auth_session_cannot_bind_callback_or_begin_effects() {
    let project = tempfile::tempdir().unwrap();
    let plugin = contribution(project.path(), PluginMcpSource::Project);
    let session = oauth::McpAuthSession {
        plugin: Some(plugin.clone()),
        caller: Some(project.path().to_path_buf()),
        server_name: "plugin".into(),
        server_url: "http://127.0.0.1:1".into(),
        auth_url: "http://127.0.0.1:1/authorize".into(),
        redirect_uri: "invalid-callback".into(),
        verifier: "fixture".into(),
        metadata: oauth::McpOAuthMetadata {
            authorization_endpoint: "http://127.0.0.1:1/authorize".into(),
            token_endpoint: "http://127.0.0.1:1/token".into(),
        },
    };
    plugin.revoke();
    let error = oauth::run_mcp_auth_session(session).await.unwrap_err();
    assert!(error.to_string().contains("revoked"), "{error:#}");
}

async fn stalled_http_withdrawal(mode: &'static str) {
    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
        sync::oneshot,
        time::{timeout, Duration},
    };
    let project = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let mut plugin = contribution(project.path(), PluginMcpSource::Project);
    plugin.config.command = None;
    plugin.config.url = Some(url);
    plugin.config.server_type = "http".into();
    let (seen, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let mut server = tokio::spawn(async move {
        let mut seen = Some(seen);
        let mut released = Box::pin(released);
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            let mut line = String::new();
            socket.read_line(&mut line).await.unwrap();
            let mut length = 0;
            loop {
                line.clear();
                socket.read_line(&mut line).await.unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut body = vec![0; length];
            socket.read_exact(&mut body).await.unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            let method = request["method"].as_str().unwrap();
            if mode != "rpc_body" || method == "tools/call" {
                match mode {
                    "sse" => socket.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap(),
                    "rpc_body" => socket.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 9999\r\nConnection: close\r\n\r\n{").await.unwrap(),
                    _ => {},
                }
                let _ = seen.take().unwrap().send(());
                let mut byte = [0];
                return tokio::select! {
                    result = socket.read(&mut byte) => result.unwrap_or(0) == 0,
                    _ = &mut released => false,
                };
            }
            if method == "notifications/initialized" {
                socket
                    .get_mut()
                    .write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
            } else {
                assert_eq!(method, "initialize");
                let body = serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}}}).to_string();
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
        }
    });
    let origin = project.path().to_path_buf();
    let connecting = plugin.clone();
    let request = tokio::spawn(async move {
        let client = McpClient::connect_plugin(&connecting, Some(&origin), None).await?;
        if mode == "rpc_body" {
            client.call_tool(Some(&origin), "blocked", None).await?;
        }
        Ok::<_, anyhow::Error>(client)
    });
    timeout(Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    plugin.revoke();
    let retirement = timeout(Duration::from_secs(2), plugin.wait_retired()).await;
    // Release the loopback fixture even on RED, so no request or worker leaks.
    let socket_closed_before_release = if retirement.is_ok() {
        let closed = timeout(Duration::from_secs(2), &mut server)
            .await
            .unwrap()
            .unwrap();
        let _ = release.send(());
        closed
    } else {
        let _ = release.send(());
        timeout(Duration::from_secs(2), &mut server)
            .await
            .unwrap()
            .unwrap()
    };
    let _ = timeout(Duration::from_secs(2), request)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(2), plugin.wait_retired())
        .await
        .unwrap()
        .unwrap();
    assert!(
        retirement.is_ok(),
        "{mode}: withdrawal must join the HTTP worker while its peer withholds a response"
    );
    retirement.unwrap().unwrap();
    assert!(
        socket_closed_before_release,
        "{mode}: local request socket must close before retirement completes"
    );
}

#[tokio::test]
async fn plugin_http_stalled_initialize_headers_retire() {
    stalled_http_withdrawal("headers").await;
}
#[tokio::test]
async fn plugin_http_stalled_initialize_sse_retire() {
    stalled_http_withdrawal("sse").await;
}
#[tokio::test]
async fn plugin_http_blocked_response_body_retires() {
    stalled_http_withdrawal("rpc_body").await;
}

#[tokio::test]
async fn plugin_subscription_receiver_closes_on_withdrawal_without_a_poller() {
    let project = tempfile::tempdir().unwrap();
    let plugin = contribution(project.path(), PluginMcpSource::Project);
    let backend = Arc::new(CountingBackend::default());
    let mut manager = McpManager::new();
    manager.clients.insert(
        "plugin".into(),
        Arc::new(McpClient::from_test_plugin_backend(
            backend.clone(),
            plugin.clone(),
        )),
    );
    manager
        .plugin_authorities
        .insert("plugin".into(), plugin.clone());
    let mut events = subscribe_resource(&manager, Some(project.path()), "plugin", "uri").await;
    assert_eq!(backend.0.load(Ordering::SeqCst), 1);
    assert!(!events.is_closed());
    plugin.revoke();
    tokio::time::timeout(std::time::Duration::from_secs(2), plugin.wait_retired())
        .await
        .unwrap()
        .unwrap();
    assert!(
        events.is_closed(),
        "withdrawal must close owned subscription channels even without a notification poller"
    );
    assert!(events.recv().await.is_none());
}

#[test]
fn saturated_plugin_subscription_retires_with_running_poller() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    const CHILD_PROJECT: &str = "COVEN_MCP_SATURATED_RETIREMENT_TEST_PROJECT";
    if let Some(project) = std::env::var_os(CHILD_PROJECT) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(saturated_plugin_subscription_child(Path::new(&project)));
        return;
    }

    // A synchronous DashMap deadlock also blocks a Tokio timeout. Bound the
    // current-thread runtime externally and join the child on either outcome.
    let project = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "live_authority_tests::saturated_plugin_subscription_retires_with_running_poller",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_PROJECT, project.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child.wait_with_output().unwrap();
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    println!(
        "saturated retirement child: timed_out={timed_out}, status={}",
        output.status
    );
    assert!(!timed_out, "saturated plugin retirement deadlocked");
    assert!(
        output.status.success(),
        "saturated plugin retirement child failed"
    );
}

async fn saturated_plugin_subscription_child(project: &Path) {
    use std::{
        io::Write,
        pin::Pin,
        task::{Context, Poll},
        time::Duration,
    };
    use tokio::{sync::oneshot, time::timeout};

    struct Notification {
        delivered: Option<oneshot::Sender<()>>,
        stopped: Option<oneshot::Sender<()>>,
    }
    impl futures::Stream for Notification {
        type Item = anyhow::Result<Value>;

        fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let Some(delivered) = self.delivered.take() else {
                return Poll::Pending;
            };
            let _ = delivered.send(());
            Poll::Ready(Some(Ok(serde_json::json!({
                "method": "notifications/resources/updated",
                "params": {"uri": "uri"}
            }))))
        }
    }
    impl Drop for Notification {
        fn drop(&mut self) {
            if let Some(stopped) = self.stopped.take() {
                let _ = stopped.send(());
            }
        }
    }

    let plugin = contribution(project, PluginMcpSource::Project);
    let (delivered, delivery_started) = oneshot::channel();
    let (stopped, stream_stopped) = oneshot::channel();
    let backend = Arc::new(CountingBackend(
        AtomicUsize::new(0),
        std::sync::Mutex::new(Some(Box::pin(Notification {
            delivered: Some(delivered),
            stopped: Some(stopped),
        }))),
    ));
    let mut manager = McpManager::new();
    manager.clients.insert(
        "plugin".into(),
        Arc::new(McpClient::from_test_plugin_backend(
            backend.clone(),
            plugin.clone(),
        )),
    );
    manager
        .plugin_authorities
        .insert("plugin".into(), plugin.clone());
    let mut events = subscribe_resource(&manager, Some(project), "plugin", "uri").await;
    let key = ("plugin".into(), "uri".into());
    let subscriptions = manager.resource_subscriptions.clone();
    {
        let sender = subscriptions.get(&key).unwrap();
        for _ in 0..events.max_capacity() {
            sender
                .try_send(ResourceChangedEvent {
                    server_name: "already-queued".into(),
                    uri: "uri".into(),
                })
                .unwrap();
        }
    }
    // Register and poll the real subscription cleanup owner before delivery.
    tokio::task::yield_now().await;
    let manager = Arc::new(manager);
    manager.clone().spawn_notification_poll_loop(Some(project));
    timeout(Duration::from_secs(2), delivery_started)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(2), async {
        while Arc::strong_count(&subscriptions) != 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(events.len(), events.max_capacity());
    // The single runtime thread cannot resume this task until the poller's
    // delivery has yielded on the full channel. Never block on the map here.
    let registry_lock_available = subscriptions.try_get_mut(&key).is_present();
    println!("SATURATED_DELIVERY_PENDING registry_lock_available={registry_lock_available}");
    std::io::stdout().flush().unwrap();
    plugin.revoke();
    timeout(Duration::from_secs(2), plugin.wait_retired())
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(2), stream_stopped)
        .await
        .unwrap()
        .unwrap();
    assert!(
        registry_lock_available,
        "pending delivery must release the subscription map guard"
    );
    assert!(!subscriptions.contains_key(&key));
    assert!(
        events.is_closed(),
        "all owned subscription senders must retire"
    );
    let mut queued = 0;
    while let Some(event) = events.recv().await {
        assert_eq!(
            event.server_name, "already-queued",
            "withdrawn delivery resumed"
        );
        queued += 1;
    }
    assert_eq!(queued, events.max_capacity());
    assert!(manager.clients["plugin"]
        .list_resources(Some(project))
        .await
        .is_err());
    assert_eq!(backend.0.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn plugin_http_rejected_initialize_joins_registered_owner() {
    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
        time::{timeout, Duration},
    };
    let project = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut plugin = contribution(project.path(), PluginMcpSource::Project);
    plugin.config.command = None;
    plugin.config.url = Some(format!("http://{}/mcp", listener.local_addr().unwrap()));
    plugin.config.server_type = "http".into();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
        let mut line = String::new();
        socket.read_line(&mut line).await.unwrap();
        let mut length = 0;
        loop {
            line.clear();
            socket.read_line(&mut line).await.unwrap();
            if line.trim().is_empty() {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut body = vec![0; length];
        socket.read_exact(&mut body).await.unwrap();
        let request: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request["method"], "initialize");
        let incorrect_id = "fixture-mismatched-response-id";
        assert_ne!(request["id"], incorrect_id);
        let body = serde_json::json!({"jsonrpc":"2.0","id":incorrect_id,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}}}).to_string();
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        socket
            .get_mut()
            .write_all(response.as_bytes())
            .await
            .unwrap();
        socket.get_mut().shutdown().await.unwrap();
    });
    let origin = project.path().to_path_buf();
    let connecting = plugin.clone();
    let mut connection =
        tokio::spawn(
            async move { McpClient::connect_plugin(&connecting, Some(&origin), None).await },
        );
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    // The complete initialize reply is deliberately rejected by rmcp's existing
    // ID validation. Cleanup must join the worker waiting for initialized.
    let failed_connection = timeout(Duration::from_secs(2), &mut connection).await;
    plugin.revoke();
    let retirement = timeout(Duration::from_secs(2), plugin.wait_retired()).await;
    if failed_connection.is_err() {
        connection.abort();
        let _ = connection.await;
    }
    assert!(
        failed_connection.is_ok(),
        "rejected initialization must finish actual worker cleanup before returning its error"
    );
    assert!(failed_connection.unwrap().unwrap().is_err());
    assert!(
        retirement.is_ok(),
        "withdrawal must join the registered failed launch owner"
    );
    retirement.unwrap().unwrap();
}

#[tokio::test]
async fn worker_close_cancels_a_pending_handler_send() {
    use rmcp::{
        service::RxJsonRpcMessage,
        transport::{
            worker::{Worker, WorkerConfig, WorkerContext, WorkerQuitReason, WorkerTransport},
            Transport,
        },
        RoleClient,
    };
    use std::{future::Future, time::Duration};
    use tokio::{sync::oneshot, time::timeout};

    struct FullHandler {
        pending: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    }
    impl Worker for FullHandler {
        type Error = std::io::Error;
        type Role = RoleClient;

        fn err_closed() -> Self::Error {
            std::io::Error::other("fixture closed")
        }
        fn err_join(error: tokio::task::JoinError) -> Self::Error {
            std::io::Error::other(error)
        }
        fn config(&self) -> WorkerConfig {
            let mut config = WorkerConfig::default();
            config.channel_buffer_capacity = 1;
            config
        }
        async fn run(
            self,
            mut context: WorkerContext<Self>,
        ) -> Result<(), WorkerQuitReason<Self::Error>> {
            let notification: RxJsonRpcMessage<RoleClient> = serde_json::from_value(serde_json::json!({
                "jsonrpc":"2.0", "method":"notifications/resources/updated", "params":{"uri":"test://pending"}
            })).unwrap();
            context.send_to_handler(notification.clone()).await?;
            let mut send = std::pin::pin!(context.send_to_handler(notification));
            let mut pending = Some(self.pending);
            tokio::select! {
                result = std::future::poll_fn(|cx| {
                    let result = send.as_mut().poll(cx);
                    if result.is_pending() {
                        if let Some(pending) = pending.take() {
                            let _ = pending.send(());
                        }
                    }
                    result
                }) => result,
                // On RED only, let the test release and join this worker after
                // recording that transport cancellation failed to close it.
                _ = self.release => Ok(()),
            }
        }
    }

    let (pending, blocked) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let mut transport = WorkerTransport::spawn(FullHandler {
        pending,
        release: released,
    });
    timeout(Duration::from_secs(2), blocked)
        .await
        .unwrap()
        .unwrap();
    let mut closing = std::pin::pin!(transport.close());
    let retirement = timeout(Duration::from_secs(2), &mut closing).await;
    let _ = release.send(());
    if retirement.is_err() {
        timeout(Duration::from_secs(2), &mut closing)
            .await
            .unwrap()
            .unwrap();
    }
    assert!(
        retirement.is_ok(),
        "close must join a worker already pending on a full handler queue"
    );
    retirement.unwrap().unwrap();
}

#[test]
fn plugin_http_drained_receive_joins_worker() {
    http_receive_retirement_subprocess("drained");
}

#[test]
fn plugin_http_saturated_receive_joins_worker() {
    http_receive_retirement_subprocess("saturated");
}

fn http_receive_retirement_subprocess(mode: &str) {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    const CHILD_MODE: &str = "COVEN_MCP_HTTP_RECEIVE_RETIREMENT_TEST_MODE";
    if std::env::var_os(CHILD_MODE).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(http_receive_retirement_child(mode));
        return;
    }
    // Bound failures outside the runtime and join the exact test subprocess.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(format!(
            "live_authority_tests::plugin_http_{mode}_receive_joins_worker"
        ))
        .args(["--nocapture", "--test-threads=1"])
        .env(CHILD_MODE, mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child.wait_with_output().unwrap();
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    println!(
        "HTTP receive retirement child: mode={mode}, timed_out={timed_out}, status={}",
        output.status
    );
    assert!(
        !timed_out,
        "HTTP retirement subprocess exceeded its external deadline"
    );
    assert!(
        output.status.success(),
        "HTTP receive retirement child failed"
    );
}

async fn http_receive_retirement_child(mode: &str) {
    use rmcp::{
        service::{RxJsonRpcMessage, TxJsonRpcMessage},
        transport::{streamable_http_client::StreamableHttpClientTransportConfig, Transport},
        RoleClient,
    };
    use std::time::Duration;
    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
        sync::{oneshot, watch},
        time::timeout,
    };

    struct Witness {
        inner: owned_http::OwnedHttpTransport,
        received: Arc<AtomicUsize>,
        closed: Option<oneshot::Sender<()>>,
    }
    impl Transport<RoleClient> for Witness {
        type Error = <owned_http::OwnedHttpTransport as Transport<RoleClient>>::Error;

        fn send(
            &mut self,
            message: TxJsonRpcMessage<RoleClient>,
        ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
            self.inner.send(message)
        }
        async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
            let message = self.inner.receive().await;
            if message.is_some() {
                self.received.fetch_add(1, Ordering::SeqCst);
            }
            message
        }
        async fn close(&mut self) -> Result<(), Self::Error> {
            println!(
                "CONCRETE_CLOSE_ENTER received={}",
                self.received.load(Ordering::SeqCst)
            );
            self.inner.close().await?;
            let _ = self.closed.take().unwrap().send(());
            Ok(())
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let (flooded, flood_ready) = oneshot::channel();
    let fixture = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            let mut line = String::new();
            socket.read_line(&mut line).await.unwrap();
            let mut length = 0;
            loop {
                line.clear();
                socket.read_line(&mut line).await.unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut body = vec![0; length];
            socket.read_exact(&mut body).await.unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            match request["method"].as_str().unwrap() {
                "initialize" => {
                    let body = serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"local-saturation","version":"1"}}}).to_string();
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    socket
                        .get_mut()
                        .write_all(response.as_bytes())
                        .await
                        .unwrap();
                }
                "notifications/initialized" => socket
                    .get_mut()
                    .write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap(),
                "tools/list" => {
                    socket.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
                    let mut burst = String::new();
                    for index in 0..1024 {
                        burst.push_str(&format!("data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/resources/updated\",\"params\":{{\"uri\":\"test://{index}\"}}}}\n\n"));
                    }
                    socket.get_mut().write_all(burst.as_bytes()).await.unwrap();
                    let _ = flooded.send(());
                    let mut byte = [0];
                    let read = socket.read(&mut byte).await;
                    println!("FIXTURE_SOCKET_END={read:?}");
                    return read;
                }
                other => panic!("unexpected fixture request {other}"),
            }
        }
    });
    let (stop, shutdown) = watch::channel(false);
    let received = Arc::new(AtomicUsize::new(0));
    let (closed, close_complete) = oneshot::channel();
    let inner = owned_http::OwnedHttpTransport::new(
        StreamableHttpClientTransportConfig::with_uri(url),
        shutdown.clone(),
    );
    let mut transport = owned_transport::OwnedTransport::new(
        Witness {
            inner,
            received: received.clone(),
            closed: Some(closed),
        },
        shutdown,
    );
    transport.send(serde_json::from_value(serde_json::json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})).unwrap()).await.unwrap();
    assert!(transport.receive().await.is_some());
    transport
        .send(
            serde_json::from_value(
                serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    transport
        .send(
            serde_json::from_value(
                serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    flood_ready.await.unwrap();
    if mode == "drained" {
        for _ in 0..1024 {
            assert!(transport.receive().await.is_some());
        }
        assert_eq!(received.load(Ordering::SeqCst), 1025);
    } else {
        timeout(Duration::from_secs(2), async {
            while received.load(Ordering::SeqCst) < 102 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // 1 initialize + 100 queued + 1 pending outer delivery. Let the real
        // HTTP worker fill its inner queue from the remaining SSE burst.
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert_eq!(received.load(Ordering::SeqCst), 102);
    }
    println!(
        "WITHDRAW mode={mode} concrete_received={}",
        received.load(Ordering::SeqCst)
    );
    stop.send_replace(true);
    let retirement = timeout(Duration::from_secs(2), transport.completion.clone()).await;
    println!("RETIREMENT={retirement:?}");
    assert!(
        retirement.is_ok(),
        "withdrawal must join the HTTP worker with saturated receive queues"
    );
    retirement.unwrap().unwrap();
    close_complete.await.unwrap();
    let socket_end = timeout(Duration::from_secs(2), fixture)
        .await
        .unwrap()
        .unwrap();
    // Closing with unread SSE bytes may reset TCP instead of delivering FIN.
    assert!(
        matches!(socket_end, Ok(0))
            || matches!(&socket_end, Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset),
        "expected socket closure, got {socket_end:?}"
    );
    println!("CONCRETE_CLOSE_COMPLETED_AND_FIXTURE_JOINED mode={mode}");
}
