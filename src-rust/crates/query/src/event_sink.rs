//! Typed event delivery with an optional connection-wide byte budget. A bounded
//! consumer which cannot keep up cancels its producer and records overload;
//! it must report that failure rather than claim complete streaming output.
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
            Channel::Unbounded(_) => false,
            Channel::Bounded { overloaded, .. } => overloaded.load(Ordering::Acquire),
        }
    }

    pub fn send(&self, event: QueryEvent) -> Result<(), EventSendError> {
        match &self.channel {
            Channel::Unbounded(sender) => sender.send(event).map_err(|_| EventSendError::Closed),
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
    async fn saturated_consumer_cancels_without_growing_or_claiming_success() {
        let cancel = CancellationToken::new();
        let budget = Arc::new(Semaphore::new(4096));
        let (sink, mut receiver) = QueryEventSink::bounded(2, budget.clone(), 4096, cancel.clone());
        for text in ["first", "second"] {
            sink.send(QueryEvent::Status(text.into())).unwrap();
        }
        assert!(sink.send(QueryEvent::Status("excess".into())).is_err());
        assert!(cancel.is_cancelled() && sink.is_overloaded());
        for text in ["first", "second"] {
            assert!(
                matches!(receiver.recv().await, Some(QueryEvent::Status(value)) if value == text)
            );
        }
        assert_eq!(budget.available_permits(), 4096);
        assert!(
            sink.send(QueryEvent::Status("later".into())).is_err(),
            "overload is latched"
        );
    }

    #[test]
    fn byte_budget_is_shared_across_producers_and_returned_on_receiver_drop() {
        let budget = Arc::new(Semaphore::new(1024));
        let (first, receiver) =
            QueryEventSink::bounded(16, budget.clone(), 1024, CancellationToken::new());
        let (second, _other) =
            QueryEventSink::bounded(16, budget.clone(), 1024, CancellationToken::new());
        first.send(QueryEvent::Status("a".repeat(500))).unwrap();
        assert!(second.send(QueryEvent::Status("b".repeat(500))).is_err());
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
            sink.send(QueryEvent::ToolQueued {
                tool_id: tool_id.clone(),
            })
            .unwrap();
            sink.send(QueryEvent::ToolRunning {
                tool_id: tool_id.clone(),
            })
            .unwrap();
            sink.send(QueryEvent::ToolFinished {
                tool_id,
                is_error: false,
            })
            .unwrap();
        }
        sink.send(QueryEvent::Status("delivered".into())).unwrap();
        assert!(!sink.is_overloaded() && !cancel.is_cancelled());
        assert!(
            matches!(receiver.recv().await, Some(QueryEvent::Status(value)) if value == "delivered")
        );
    }

    #[test]
    fn oversized_event_is_not_queued() {
        let budget = Arc::new(Semaphore::new(1024));
        let cancel = CancellationToken::new();
        let (sink, _receiver) = QueryEventSink::bounded(16, budget.clone(), 1024, cancel.clone());
        assert!(sink.send(QueryEvent::Status("x".repeat(2048))).is_err());
        assert!(sink.is_overloaded() && cancel.is_cancelled());
        assert_eq!(budget.available_permits(), 1024);
    }
}
