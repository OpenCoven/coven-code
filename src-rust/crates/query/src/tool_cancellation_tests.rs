// Included inside the existing query tests so both actual dispatcher routes
// share the same typed provider fixtures as the rest of the query suite.
#[cfg(unix)]
async fn cancelled_shell_stops_before_delayed_write(provider_route: bool, pty: bool) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = tempfile::tempdir().unwrap();
    let started = dir.path().join("started");
    let late = dir.path().join("late");
    let command = format!(
        "sh -c 'printf started > \"{}\"; sleep 1; printf late > \"{}\"'",
        started.display(), late.display()
    );
    let call = ContentBlock::ToolUse {
        id: "cancelled-shell".into(), name: "Bash".into(),
        input: serde_json::json!({"command":command}),
    };
    let tools: Vec<Box<dyn Tool>> = if pty {
        vec![Box::new(claurst_tools::PtyBashTool)]
    } else {
        vec![Box::new(claurst_tools::BashTool)]
    };
    let mut config = make_config(None, None);
    config.max_turns = 1;
    let mut server = None;
    let mut client_config = claurst_api::client::ClientConfig {
        api_key: "synthetic-shell-cancel-key".into(), api_base: "http://127.0.0.1:9".into(), max_retries: 0,
        ..Default::default()
    };
    if provider_route {
        let mut registry = claurst_api::ProviderRegistry::new();
        registry.register(Arc::new(ScriptedProvider::new(vec![scripted_round(
            "tool-cancel", vec![call.clone()], ProviderStopReason::ToolUse,
        )])));
        config.model = "scripted".into();
        config.provider_registry = Some(Arc::new(registry));
    } else {
        let ContentBlock::ToolUse { input, .. } = &call else { unreachable!() };
        let events = [
            ("message_start", serde_json::json!({"message":{"id":"tool-cancel","model":"fixture","usage":{"input_tokens":1,"output_tokens":0}}})),
            ("content_block_start", serde_json::json!({"index":0,"content_block":{"type":"tool_use","id":"cancelled-shell","name":"Bash","input":{}}})),
            ("content_block_delta", serde_json::json!({"index":0,"delta":{"type":"input_json_delta","partial_json":input.to_string()}})),
            ("content_block_stop", serde_json::json!({"index":0})),
            ("message_delta", serde_json::json!({"delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":1}})),
            ("message_stop", serde_json::json!({})),
        ];
        let body: String = events.into_iter().map(|(event,data)|format!("event: {event}\ndata: {data}\n\n")).collect();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        client_config.api_base = format!("http://{}",listener.local_addr().unwrap());
        server = Some(tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(std::time::Duration::from_secs(5),listener.accept()).await.unwrap().unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0u8;4096];
                let n = tokio::time::timeout(std::time::Duration::from_secs(5),stream.read(&mut chunk)).await.unwrap().unwrap();
                assert!(n>0,"request closed early");request.extend_from_slice(&chunk[..n]);assert!(request.len()<1024*1024);
                if let Some(end) = request.windows(4).position(|w|w==b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    let length: usize = header.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|s|s.trim().parse().unwrap())).unwrap();
                    if request.len()>=end+4+length {break;}
                }
            }
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            stream.write_all(response.as_bytes()).await.unwrap();stream.shutdown().await.unwrap();
        }));
    }
    let client = claurst_api::AnthropicClient::new(client_config).unwrap();
    let costs = CostTracker::new();
    let mut ctx = scheduling_ctx(dir.path());ctx.cost_tracker = costs.clone();ctx.session_id = uuid::Uuid::new_v4().to_string();
    if provider_route { ctx.config.provider = Some(SCRIPTED_PROVIDER_ID.into()); }
    let token = tokio_util::sync::CancellationToken::new();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut messages = vec![Message::user("run the bounded shell fixture")];
    let run = run_query_loop(&client,&mut messages,&tools,&ctx,&config,costs,Some(tx),token.clone(),None);
    let trigger = async {
        tokio::time::timeout(std::time::Duration::from_secs(5),async {
            while !started.exists() {tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
        }).await.expect("real shell worker must start");
        token.cancel();
    };
    let (outcome,()) = tokio::time::timeout(std::time::Duration::from_secs(8),async {tokio::join!(run,trigger)}).await.expect("cancelled query must settle");
    if let Some(server) = server {server.await.unwrap();}
    assert!(matches!(outcome,QueryOutcome::Cancelled),"{outcome:?}");
    assert!(!late.exists(),"cancelled running shell still performed its delayed write");
    // Waiting beyond the worker's original delay also rejects a merely early
    // query return that left its foreground process alive.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert!(!late.exists(),"cancel returned before owned shell cleanup");
    let mut events = Vec::new();while let Ok(event)=rx.try_recv(){events.push(event);}
    assert_unexecuted_tool_result_is_durable_once(&events,&messages,"cancelled-shell");
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_query_stops_provider_bash() { cancelled_shell_stops_before_delayed_write(true,false).await; }
#[cfg(unix)]
#[tokio::test]
async fn cancelled_query_stops_provider_pty() { cancelled_shell_stops_before_delayed_write(true,true).await; }
#[cfg(unix)]
#[tokio::test]
async fn cancelled_query_stops_anthropic_bash() { cancelled_shell_stops_before_delayed_write(false,false).await; }
#[cfg(unix)]
#[tokio::test]
async fn cancelled_query_stops_anthropic_pty() { cancelled_shell_stops_before_delayed_write(false,true).await; }

struct CancellationWriter;

#[async_trait]
impl Tool for CancellationWriter {
    fn name(&self) -> &str { "CancellationWriter" }
    fn description(&self) -> &str { "Controlled writer cancellation fixture" }
    fn permission_level(&self) -> PermissionLevel { PermissionLevel::Write }
    fn input_schema(&self) -> Value { serde_json::json!({"type":"object"}) }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> ToolResult {
        let path = std::path::Path::new(input["directory"].as_str().unwrap());
        match input["phase"].as_str().unwrap() {
            "complete" => {
                std::fs::write(path.join("completed"), "acknowledged").unwrap();
                ToolResult::success("acknowledged completed write")
            }
            "running" => {
                std::fs::write(path.join("running"), "started").unwrap();
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                std::fs::write(path.join("late"), "must not run").unwrap();
                ToolResult::success("finished too late")
            }
            "queued" => {
                std::fs::write(path.join("queued"), "must not run").unwrap();
                ToolResult::success("queued write ran")
            }
            _ => ToolResult::error("unknown fixture phase"),
        }
    }
}

#[tokio::test]
async fn cancelled_query_retains_completed_results_and_stops_queued_writers() {
    let dir = tempfile::tempdir().unwrap();
    let calls = ["complete","running","queued"].map(|phase|ContentBlock::ToolUse {
        id:phase.into(),name:"CancellationWriter".into(),input:serde_json::json!({"phase":phase,"directory":dir.path()}),
    }).to_vec();
    let provider = ScriptedProvider::new(vec![scripted_round("cancel-writers",calls,ProviderStopReason::ToolUse)]);
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(CancellationWriter)];
    let token = tokio_util::sync::CancellationToken::new();
    let run = run_scripted_provider_with_token(provider,make_config(None,None),Config {
        provider:Some(SCRIPTED_PROVIDER_ID.into()),..Config::default()
    },&tools,token.clone());
    let cancel = async {
        tokio::time::timeout(std::time::Duration::from_secs(5),async {
            while !dir.path().join("running").exists() {tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
        }).await.expect("second writer must start after first completes");
        token.cancel();
    };
    let ((outcome,events,messages,_),()) = tokio::time::timeout(std::time::Duration::from_secs(8),async {tokio::join!(run,cancel)}).await.unwrap();
    assert!(matches!(outcome,QueryOutcome::Cancelled),"{outcome:?}");
    assert!(dir.path().join("completed").exists());
    assert!(!dir.path().join("late").exists(),"running writer continued after cancellation");
    assert!(!dir.path().join("queued").exists(),"queued writer started after cancellation");
    let results:Vec<_> = messages.iter().flat_map(|message|match &message.content {
        MessageContent::Blocks(blocks)=>blocks.iter().filter(|b|matches!(b,ContentBlock::ToolResult {..})).collect(),_=>Vec::new()
    }).cloned().collect();
    assert_eq!(results.len(),3,"one final result per original call");
    assert!(!result_for(&results,"complete").1,"completed success was discarded");
    assert!(result_for(&results,"running").1);
    assert!(result_for(&results,"queued").1);
    for id in ["running","queued"] {
        let ContentBlock::ToolResult {content:ToolResultContent::Text(text),..} = result_for(&results,id).0 else {panic!("expected typed result text")};
        assert!(text.contains("cancelled") && text.contains("side effects"),"{text}");
    }
    assert!(events.iter().all(|event|!matches!(event,QueryEvent::TurnComplete {..})));
    assert_eq!(events.iter().filter(|event|matches!(event,QueryEvent::DurableMessage {message} if is_tool_result_carrier(message))).count(),1);
}
