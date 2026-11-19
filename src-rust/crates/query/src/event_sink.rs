//! Typed event delivery. Three shapes:
//! - unbounded, for consumers that print as fast as events arrive (headless);
//! - backpressured, where the producer waits for queue space, so a slow
//!   consumer pauses the turn instead of growing a buffer and nothing is
//!   dropped (the interactive terminal);
//! - bounded with a connection-wide byte budget, where a consumer which cannot
//!   keep up cancels its producer and records overload; it must report that
//!   failure rather than claim complete streaming output (ACP).
use std::fmt::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::QueryEvent;

#[derive(Debug)]
pub enum EventSendError {
    Closed,
    Capacity,
}

#[derive(Clone)]
pub struct QueryEventSink {
    channel: Channel,
}

#[derive(Clone)]
enum Channel {
    Unbounded(mpsc::UnboundedSender<QueryEvent>),
    Backpressured(mpsc::Sender<QueryEvent>),
    Bounded {
        sender: mpsc::Sender<(QueryEvent, OwnedSemaphorePermit)>,
        budget: Arc<Semaphore>,
        cancel: CancellationToken,
        overloaded: Arc<AtomicBool>,
        max_bytes: usize,
    },
}

pub struct BoundedEventReceiver {
    receiver: mpsc::Receiver<(QueryEvent, OwnedSemaphorePermit)>,
}

impl BoundedEventReceiver {
    pub async fn recv(&mut self) -> Option<QueryEvent> {
        self.receiver.recv().await.map(|(event, _permit)| event)
    }
}

impl From<mpsc::UnboundedSender<QueryEvent>> for QueryEventSink {
    fn from(sender: mpsc::UnboundedSender<QueryEvent>) -> Self {
        Self {
            channel: Channel::Unbounded(sender),
        }
    }
}

/// The producer waits for space in the channel; it never drops an event.
impl From<mpsc::Sender<QueryEvent>> for QueryEventSink {
    fn from(sender: mpsc::Sender<QueryEvent>) -> Self {
        Self {
            channel: Channel::Backpressured(sender),
        }
    }
}

impl QueryEventSink {
    pub fn bounded(
        capacity: usize,
        budget: Arc<Semaphore>,
        max_bytes: usize,
        cancel: CancellationToken,
    ) -> (Self, BoundedEventReceiver) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                channel: Channel::Bounded {
                    sender,
                    budget,
                    cancel,
                    overloaded: Arc::new(AtomicBool::new(false)),
                    max_bytes,
                },
            },
            BoundedEventReceiver { receiver },
        )
    }

    pub fn is_overloaded(&self) -> bool {
        match &self.channel {
            Channel::Unbounded(_) | Channel::Backpressured(_) => false,
            Channel::Bounded { overloaded, .. } => overloaded.load(Ordering::Acquire),
        }
    }

    /// Deliver one event. Only a backpressured sink waits, and only while its
    /// channel is full; the other shapes complete without suspending.
    pub async fn emit(&self, event: QueryEvent) -> Result<(), EventSendError> {
        match &self.channel {
            Channel::Unbounded(sender) => sender.send(event).map_err(|_| EventSendError::Closed),
            Channel::Backpressured(sender) => {
                sender.send(event).await.map_err(|_| EventSendError::Closed)
            }
            Channel::Bounded {
                sender,
                budget,
                cancel,
                overloaded,
                max_bytes,
            } => {
                // Queue and finish progress only drives a local display. A
                // bounded consumer relays to a remote client, where these would
                // take slots and budget meant for events it must deliver.
                if matches!(
                    event,
                    QueryEvent::ToolQueued { .. }
                        | QueryEvent::ToolRunning { .. }
                        | QueryEvent::ToolFinished { .. }
                ) {
                    return Ok(());
                }
                if overloaded.load(Ordering::Acquire) {
                    return Err(EventSendError::Capacity);
                }
                // Count all dynamically sized fields without allocating a debug
                // string, stopping as soon as a single event exceeds the budget.
                let mut size = EventSize {
                    bytes: 256,
                    limit: *max_bytes,
                };
                let fits = write!(&mut size, "{event:?}").is_ok();
                let permit = fits
                    .then(|| u32::try_from(size.bytes).ok())
                    .flatten()
                    .and_then(|bytes| budget.clone().try_acquire_many_owned(bytes).ok());
                let Some(permit) = permit else {
                    overloaded.store(true, Ordering::Release);
                    cancel.cancel();
                    return Err(EventSendError::Capacity);
                };
                match sender.try_send((event, permit)) {
                    Ok(()) => Ok(()),
                    Err(mpsc::error::TrySendError::Full((_event, _permit))) => {
                        overloaded.store(true, Ordering::Release);
                        cancel.cancel();
                        Err(EventSendError::Capacity)
                    }
                    Err(mpsc::error::TrySendError::Closed((_event, _permit))) => {
                        cancel.cancel();
                        Err(EventSendError::Closed)
                    }
                }
            }
        }
    }
}

struct EventSize {
    bytes: usize,
    limit: usize,
}
impl fmt::Write for EventSize {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.bytes = self.bytes.saturating_add(text.len());
        if self.bytes > self.limit {
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backpressured_sink_waits_for_space_and_never_drops() {
        let (sender, mut receiver) = mpsc::channel(1);
        let sink = QueryEventSink::from(sender);
        sink.emit(QueryEvent::Status("first".into())).await.unwrap();
        let second = sink.emit(QueryEvent::Status("second".into()));
        tokio::pin!(second);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut second)
                .await
                .is_err(),
            "a full sink waits instead of dropping or queueing more"
        );
        assert!(!sink.is_overloaded());
        assert!(matches!(receiver.recv().await, Some(QueryEvent::Status(text)) if text == "first"));
        second.await.unwrap();
        assert!(
            matches!(receiver.recv().await, Some(QueryEvent::Status(text)) if text == "second")
        );
        drop(receiver);
        assert!(matches!(
            sink.emit(QueryEvent::Status("orphan".into())).await,
            Err(EventSendError::Closed)
        ));
    }

    #[tokio::test]
    async fn saturated_consumer_cancels_without_growing_or_claiming_success() {
        let cancel = CancellationToken::new();
        let budget = Arc::new(Semaphore::new(4096));
        let (sink, mut receiver) = QueryEventSink::bounded(2, budget.clone(), 4096, cancel.clone());
        for text in ["first", "second"] {
            sink.emit(QueryEvent::Status(text.into())).await.unwrap();
        }
        assert!(sink
            .emit(QueryEvent::Status("excess".into()))
            .await
            .is_err());
        assert!(cancel.is_cancelled() && sink.is_overloaded());
        for text in ["first", "second"] {
            assert!(
                matches!(receiver.recv().await, Some(QueryEvent::Status(value)) if value == text)
            );
        }
        assert_eq!(budget.available_permits(), 4096);
        assert!(
            sink.emit(QueryEvent::Status("later".into())).await.is_err(),
            "overload is latched"
        );
    }

    #[tokio::test]
    async fn byte_budget_is_shared_across_producers_and_returned_on_receiver_drop() {
        let budget = Arc::new(Semaphore::new(1024));
        let (first, receiver) =
            QueryEventSink::bounded(16, budget.clone(), 1024, CancellationToken::new());
        let (second, _other) =
            QueryEventSink::bounded(16, budget.clone(), 1024, CancellationToken::new());
        first
            .emit(QueryEvent::Status("a".repeat(500)))
            .await
            .unwrap();
        assert!(second
            .emit(QueryEvent::Status("b".repeat(500)))
            .await
            .is_err());
        assert!(second.is_overloaded());
        drop(receiver);
        assert_eq!(budget.available_permits(), 1024);
    }

    #[tokio::test]
    async fn display_progress_never_takes_a_bounded_slot() {
        let budget = Arc::new(Semaphore::new(4096));
        let cancel = CancellationToken::new();
        let (sink, mut receiver) = QueryEventSink::bounded(1, budget.clone(), 4096, cancel.clone());
        for _ in 0..8 {
            let tool_id = "toolu_1".to_string();
            sink.emit(QueryEvent::ToolQueued {
                tool_id: tool_id.clone(),
            })
            .await
            .unwrap();
            sink.emit(QueryEvent::ToolRunning {
                tool_id: tool_id.clone(),
            })
            .await
            .unwrap();
            sink.emit(QueryEvent::ToolFinished {
                tool_id,
                is_error: false,
            })
            .await
            .unwrap();
        }
        sink.emit(QueryEvent::Status("delivered".into()))
            .await
            .unwrap();
        assert!(!sink.is_overloaded() && !cancel.is_cancelled());
        assert!(
            matches!(receiver.recv().await, Some(QueryEvent::Status(value)) if value == "delivered")
        );
    }

    #[tokio::test]
    async fn oversized_event_is_not_queued() {
        let budget = Arc::new(Semaphore::new(1024));
        let cancel = CancellationToken::new();
        let (sink, _receiver) = QueryEventSink::bounded(16, budget.clone(), 1024, cancel.clone());
        assert!(sink
            .emit(QueryEvent::Status("x".repeat(2048)))
            .await
            .is_err());
        assert!(sink.is_overloaded() && cancel.is_cancelled());
        assert_eq!(budget.available_permits(), 1024);
    }
}
