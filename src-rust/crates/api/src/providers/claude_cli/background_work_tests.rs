// A run in which the CLI starts background work prints several `result`
// lines. These tests drive the provider with a stand-in `claude`: a shell
// script that prints the lines the real CLI prints, in the order it prints
// them (recorded from Claude Code 2.1.326).

use super::tests::{fixture_binary, request_with};
use super::*;
use futures::StreamExt;
use std::time::{Duration, Instant};

const INIT: &str = r#"echo '{"type":"system","subtype":"init","model":"fixture"}'"#;
const TASK_RUNNING: &str =
    r#"echo '{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"t1"}]}'"#;
const NO_TASKS: &str =
    r#"echo '{"type":"system","subtype":"background_tasks_changed","tasks":[]}'"#;
const WAITING: &str = r#"echo '{"type":"assistant","message":{"content":[{"type":"text","text":"It is running. I will report when it finishes."}]}}'"#;
const FIRST_RESULT: &str =
    r#"echo '{"type":"result","is_error":false,"usage":{"input_tokens":10,"output_tokens":5}}'"#;
const ANSWER: &str =
    r#"echo '{"type":"assistant","message":{"content":[{"type":"text","text":"slept"}]}}'"#;
const LAST_RESULT: &str =
    r#"echo '{"type":"result","is_error":false,"usage":{"input_tokens":1,"output_tokens":2}}'"#;

/// Runs one turn against a stand-in `claude` whose script is `lines`. Reads
/// the stream the way the query loop does, up to its `MessageStop`, and
/// returns the events with how long they took.
async fn run(lines: &[&str], heartbeat: Duration) -> (Vec<StreamEvent>, Duration) {
    let dir = std::env::temp_dir().join(format!("background-claude-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let script = format!("#!/bin/sh\ncat >/dev/null\n{}\n", lines.join("\n"));
    let binary = fixture_binary(&dir, &script);
    let provider = ClaudeCliProvider {
        binary: Some(binary),
        heartbeat,
        ..ClaudeCliProvider::new()
    };
    let request = request_with(vec![Message::user("fixture prompt")]);
    let started = Instant::now();
    let mut stream = provider.create_message_stream(request).await.unwrap();
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.unwrap());
        if matches!(events.last(), Some(StreamEvent::MessageStop)) {
            break;
        }
    }
    let took = started.elapsed();
    drop(stream);
    let _ = std::fs::remove_dir_all(dir);
    (events, took)
}

fn texts(events: &[StreamEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The stream closes once, at its end, with the usage of every result.
fn assert_closes_once_with(events: &[StreamEvent], input_tokens: u64, output_tokens: u64) {
    let closings: Vec<&StreamEvent> = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                StreamEvent::MessageDelta {
                    stop_reason: Some(_),
                    ..
                } | StreamEvent::MessageStop
            )
        })
        .collect();
    assert!(
        matches!(
            closings.as_slice(),
            [
                StreamEvent::MessageDelta {
                    stop_reason: Some(StopReason::EndTurn),
                    usage: Some(usage),
                },
                StreamEvent::MessageStop,
            ] if usage.input_tokens == input_tokens && usage.output_tokens == output_tokens
        ),
        "{closings:?}"
    );
    assert!(matches!(events.last(), Some(StreamEvent::MessageStop)));
}

/// The failure this guards against: the CLI reports a result while its
/// background command is still running, and the answer comes a turn later.
/// Ending at the first result loses the answer and kills the command.
#[tokio::test]
async fn a_result_printed_while_background_work_runs_does_not_end_the_stream() {
    let (events, _) = run(
        &[
            INIT,
            TASK_RUNNING,
            WAITING,
            FIRST_RESULT,
            "sleep 1",
            NO_TASKS,
            INIT,
            ANSWER,
            LAST_RESULT,
        ],
        Duration::from_millis(100),
    )
    .await;

    assert_eq!(
        texts(&events),
        ["It is running. I will report when it finishes.", "slept"]
    );
    assert_closes_once_with(&events, 11, 7);
    // The wait is quiet, and the CLI is alive: keepalives cover it.
    let first_text = events
        .iter()
        .position(|event| matches!(event, StreamEvent::TextDelta { .. }))
        .unwrap();
    let pings_while_waiting = events[first_text..]
        .iter()
        .filter(|event| matches!(event, StreamEvent::Ping))
        .count();
    assert!(
        pings_while_waiting >= 5,
        "{pings_while_waiting} keepalives in one second of background work"
    );
}

/// The command can finish while the model is still writing "I will report".
/// That result arrives with nothing left running, and the report still
/// follows it.
#[tokio::test]
async fn work_that_ends_before_the_result_still_gets_its_report() {
    let (events, _) = run(
        &[
            INIT,
            TASK_RUNNING,
            NO_TASKS,
            WAITING,
            FIRST_RESULT,
            "sleep 1",
            INIT,
            ANSWER,
            LAST_RESULT,
        ],
        // Four seconds for the next turn to start; it starts after one.
        Duration::from_secs(2),
    )
    .await;

    assert_eq!(texts(&events).last(), Some(&"slept"));
    assert_closes_once_with(&events, 11, 7);
}

/// The turn that reports on the work can be quiet for a while, as any turn
/// can. Having started, it is waited for like any other.
#[tokio::test]
async fn a_report_that_has_started_is_waited_for() {
    let (events, _) = run(
        &[
            INIT,
            TASK_RUNNING,
            WAITING,
            FIRST_RESULT,
            NO_TASKS,
            INIT,
            "sleep 1",
            ANSWER,
            LAST_RESULT,
        ],
        Duration::from_millis(100),
    )
    .await;

    assert_eq!(texts(&events).last(), Some(&"slept"));
    assert_closes_once_with(&events, 11, 7);
}

/// A CLI that gives up on its background work exits after the result it
/// already printed. That is the end of the run, not a failure.
#[tokio::test]
async fn a_cli_that_exits_with_work_unreported_ends_the_stream_with_what_it_said() {
    let (events, _) = run(
        &[INIT, TASK_RUNNING, WAITING, FIRST_RESULT],
        Duration::from_secs(30),
    )
    .await;

    assert_eq!(
        texts(&events),
        ["It is running. I will report when it finishes."]
    );
    assert_closes_once_with(&events, 10, 5);
}

/// Ending at the CLI's exit must not mean waiting on a CLI that has said
/// everything and stays open.
#[tokio::test]
async fn a_cli_that_stays_open_after_its_last_result_is_not_waited_for() {
    let (events, took) = run(
        &[
            INIT,
            TASK_RUNNING,
            WAITING,
            FIRST_RESULT,
            NO_TASKS,
            INIT,
            ANSWER,
            LAST_RESULT,
            "exec sleep 30",
        ],
        Duration::from_millis(100),
    )
    .await;

    assert_eq!(texts(&events).last(), Some(&"slept"));
    assert_closes_once_with(&events, 11, 7);
    assert!(took < Duration::from_secs(10), "the stream took {took:?}");
}

/// A run with no background work is unchanged: its result ends it, whether
/// or not the CLI then exits.
#[tokio::test]
async fn a_run_without_background_work_ends_at_its_result() {
    let (events, took) = run(
        &[INIT, ANSWER, LAST_RESULT, "exec sleep 30"],
        Duration::from_secs(30),
    )
    .await;

    assert_eq!(texts(&events), ["slept"]);
    assert_closes_once_with(&events, 1, 2);
    assert!(took < Duration::from_secs(10), "the stream took {took:?}");
}

/// An error result ends the stream at once, background work or not.
#[test]
fn an_error_result_ends_the_stream_while_background_work_runs() {
    let mut mapper = EnvelopeMapper::new("fixture");
    mapper.map_line(
        r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"t1"}]}"#,
    );
    let outcome = mapper.map_line(
        r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"limit reached"}"#,
    );
    assert!(outcome.done);
    assert!(outcome.events.iter().any(|event| matches!(
        event,
        StreamEvent::Error { message, .. } if message == "limit reached"
    )));
    assert!(matches!(
        outcome.events.last(),
        Some(StreamEvent::MessageStop)
    ));
}
