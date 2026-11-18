//! Adversarial combined-background/silence lifecycle regressions.
use super::tests::{fixture_binary, request_with};
use super::*;
use futures::StreamExt;
use std::time::{Duration, Instant};

struct Fixture {
    dir: PathBuf,
    provider: ClaudeCliProvider,
}
impl Fixture {
    fn new(body: &str, heartbeat: Duration, limit: Option<Duration>) -> Self {
        let dir = std::env::temp_dir().join(format!("claude-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let binary = fixture_binary(&dir, &format!("#!/bin/sh\ncat >/dev/null\n{body}\n"));
        let provider = ClaudeCliProvider {
            binary: Some(binary),
            heartbeat,
            quiet_limit: limit,
            ..ClaudeCliProvider::new()
        };
        Self { dir, provider }
    }
    async fn collect(&self) -> Vec<Result<StreamEvent, ProviderError>> {
        let request = request_with(vec![Message::user("fixture prompt")]);
        let mut stream = self.provider.create_message_stream(request).await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            let terminal = matches!(
                event,
                Ok(StreamEvent::MessageStop | StreamEvent::Error { .. }) | Err(_)
            );
            events.push(event);
            if terminal {
                break;
            }
        }
        events
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
const INIT: &str = r#"echo '{"type":"system","subtype":"init","model":"fixture"}'"#;
const TASK: &str =
    r#"echo '{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"t1"}]}'"#;
const NO_TASK: &str = r#"echo '{"type":"system","subtype":"background_tasks_changed","tasks":[]}'"#;
const RESULT: &str = r#"echo '{"type":"result","is_error":false,"usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":3,"cache_read_input_tokens":7}}'"#;
const ANSWER: &str =
    r#"echo '{"type":"assistant","message":{"content":[{"type":"text","text":"answer"}]}}'"#;

#[tokio::test]
async fn internal_progress_does_not_suppress_consumer_keepalives() {
    let body = format!("{INIT}\n{TASK}\n{RESULT}\nfor _ in $(seq 30); do sleep 0.03; {TASK}; done\n{ANSWER}\n{RESULT}");
    let f = Fixture::new(
        &body,
        Duration::from_millis(100),
        Some(Duration::from_secs(2)),
    );
    let events = f.collect().await;
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    let pings = events
        .iter()
        .filter(|e| matches!(e, Ok(StreamEvent::Ping)))
        .count();
    assert!(pings >= 5, "internal stdout suppressed keepalives: {pings}");
}

#[tokio::test]
async fn a_short_quiet_deadline_does_not_wait_for_the_heartbeat() {
    let f = Fixture::new(
        &format!("{INIT}\nsleep 1\n{ANSWER}\n{RESULT}"),
        Duration::from_secs(5),
        Some(Duration::from_millis(200)),
    );
    let events = f.collect().await;
    assert!(
        matches!(events.last(), Some(Err(ProviderError::StreamError { message, .. })) if message.contains("printed nothing")),
        "{events:?}"
    );
    assert!(!events
        .iter()
        .any(|e| matches!(e, Ok(StreamEvent::TextDelta { .. }))));
}

#[tokio::test]
async fn stdout_closed_before_startup_cannot_bypass_the_quiet_deadline() {
    let f = Fixture::new(
        "exec 1>&-; exec sleep 2",
        Duration::from_millis(100),
        Some(Duration::from_millis(200)),
    );
    let request = request_with(vec![Message::user("fixture prompt")]);
    let result = f.provider.create_message_stream(request).await;
    assert!(
        matches!(result, Err(ProviderError::StreamError { message, .. }) if message.contains("printed nothing"))
    );
}

#[tokio::test]
async fn an_unfinished_report_is_not_an_earlier_turns_success() {
    for exit in [0, 7] {
        let f = Fixture::new(&format!("{INIT}\n{TASK}\n{RESULT}\n{NO_TASK}\n{INIT}\n{ANSWER}\necho report-failed >&2\nexit {exit}"), Duration::from_millis(100), None);
        let events = f.collect().await;
        assert!(
            matches!(events.last(), Some(Err(ProviderError::StreamError { .. }))),
            "exit={exit}: {events:?}"
        );
        assert!(!events
            .iter()
            .any(|e| matches!(e, Ok(StreamEvent::MessageStop))));
    }
}

#[tokio::test]
async fn failed_cli_exit_after_a_result_is_not_success() {
    let f = Fixture::new(
        &format!("{INIT}\n{TASK}\n{RESULT}\nexit 7"),
        Duration::from_millis(100),
        None,
    );
    let events = f.collect().await;
    assert!(
        matches!(events.last(), Some(Err(ProviderError::StreamError { .. }))),
        "{events:?}"
    );
}

#[tokio::test]
async fn completed_results_publish_usage_before_a_later_timeout() {
    let f = Fixture::new(
        &format!("{INIT}\n{TASK}\n{RESULT}\nexec sleep 2"),
        Duration::from_millis(50),
        Some(Duration::from_millis(200)),
    );
    let events = f.collect().await;
    let mut accumulator = crate::StreamAccumulator::new();
    for event in events.iter().flatten() {
        accumulator.on_provider_event(event);
    }
    let (_, usage, _) = accumulator.finish();
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_creation_input_tokens,
            usage.cache_read_input_tokens
        ),
        (10, 5, 3, 7)
    );
    assert!(events.last().unwrap().is_err(), "{events:?}");
}

#[tokio::test]
async fn an_error_result_publishes_its_usage_before_the_error() {
    let f = Fixture::new(&format!("{INIT}\necho '{{\"type\":\"result\",\"is_error\":true,\"result\":\"fixture failure\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":5}}}}'"), Duration::from_millis(100), None);
    let events = f.collect().await;
    let mut accumulator = crate::StreamAccumulator::new();
    for event in events.iter().flatten() {
        accumulator.on_provider_event(event);
    }
    let (_, usage, _) = accumulator.finish();
    assert_eq!((usage.input_tokens, usage.output_tokens), (10, 5));
    assert!(matches!(events.last(), Some(Ok(StreamEvent::Error { .. }))));
}

#[tokio::test]
async fn unrelated_metadata_does_not_extend_the_exit_grace() {
    let body = format!("sleep 1.2\n{INIT}\n{TASK}\n{RESULT}\n{NO_TASK}\nfor _ in $(seq 40); do sleep 0.05; {NO_TASK}; done\nexec sleep 2");
    let f = Fixture::new(&body, Duration::from_millis(100), None);
    let request = request_with(vec![Message::user("fixture prompt")]);
    let mut stream = f.provider.create_message_stream(request).await.unwrap();
    // The grace belongs to the reporting phase, after the startup handshake.
    let start = Instant::now();
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        let terminal = matches!(
            event,
            Ok(StreamEvent::MessageStop | StreamEvent::Error { .. }) | Err(_)
        );
        events.push(event);
        if terminal {
            break;
        }
    }
    drop(stream);
    assert!(
        matches!(events.last(), Some(Ok(StreamEvent::MessageStop))),
        "{events:?}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "metadata extended grace: {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn a_persisted_goal_turn_preserves_the_quiet_limit() {
    use claurst_core::goal::{GoalStore, GoalTurnPermission};
    let f = Fixture::new(
        &format!("{INIT}\nexec sleep 2"),
        Duration::from_millis(50),
        Some(Duration::from_millis(200)),
    );
    let store = GoalStore::open(&f.dir.join("goals.sqlite")).unwrap();
    store
        .set_goal("fixture", "Exercise the persisted goal provider", None)
        .unwrap();
    let turn = store.capture_active_goal("fixture").unwrap().unwrap();
    let request = request_with(vec![Message::user("fixture prompt")]);
    let mut stream = f
        .provider
        .create_message_stream_for_goal(request, Some(&turn), GoalTurnPermission::Work)
        .await
        .unwrap();
    let mut failed = false;
    while let Some(event) = stream.next().await {
        if matches!(event, Err(ProviderError::StreamError { message, .. }) if message.contains("printed nothing"))
        {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert!(turn.request_attempted());
    assert_eq!(turn.validate().unwrap(), GoalTurnPermission::Work);
}
