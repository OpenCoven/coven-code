// command_queue.rs — T1-4: Command Queue Draining
//
// A priority queue shared between the TUI input thread and the query loop.
// Commands are drained at the start of each turn, before the API call.
//
// Mirrors the TypeScript `messageQueueManager.js` behaviour.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPriority {
    Interrupt = 3,
    High = 2,
    Normal = 1,
    Low = 0,
}

impl PartialOrd for CommandPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CommandPriority {
    fn cmp(&self, other: &Self) -> Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

// ---------------------------------------------------------------------------
// Command variants
// ---------------------------------------------------------------------------

/// Commands that can be enqueued by the TUI or other components.
#[derive(Debug, Clone)]
pub enum QueuedCommand {
    /// /compact — compress conversation context
    Compact,
    /// /clear — reset conversation history
    Clear,
    /// Change the active model
    SetModel(String),
    /// Inject a plain user message into the conversation
    InjectUserMessage(String),
    /// Inject a system-level message (sent as a user message with [System] prefix)
    InjectSystemMessage(String),
    /// Trigger a named skill
    TriggerSkill(String),
}

// ---------------------------------------------------------------------------
// CommandQueue
// ---------------------------------------------------------------------------

/// Thread-safe priority queue of commands.
///
/// Cloning the handle yields a second handle to the *same* queue (Arc semantics).
#[derive(Debug, Clone)]
pub struct CommandQueue(Arc<Mutex<[VecDeque<QueuedCommand>; 4]>>);

impl CommandQueue {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(std::array::from_fn(|_| {
            VecDeque::new()
        }))))
    }

    /// Push a command with the given priority.
    pub fn push(&self, command: QueuedCommand, priority: CommandPriority) {
        self.0.lock().unwrap()[priority as usize].push_back(command);
    }

    /// Drain all pending commands in priority order (highest first), FIFO within each priority.
    pub fn drain(&self) -> Vec<QueuedCommand> {
        let mut buckets = self.0.lock().unwrap();
        let mut out = Vec::with_capacity(buckets.iter().map(VecDeque::len).sum());
        for bucket in buckets.iter_mut().rev() {
            out.extend(bucket.drain(..));
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().iter().all(VecDeque::is_empty)
    }
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// drain_command_queue
// ---------------------------------------------------------------------------

/// Consume all pending commands and convert them to `Message`s that can be
/// prepended to the conversation before the next API call.
///
/// Commands that are handled purely by the TUI/app layer (Compact, Clear,
/// SetModel, TriggerSkill) are silently dropped here — the query loop does
/// not need to act on them directly.
pub fn drain_command_queue(queue: &CommandQueue) -> Vec<claurst_core::types::Message> {
    use claurst_core::types::Message;

    let commands = queue.drain();
    let mut messages = Vec::new();

    for cmd in commands {
        match cmd {
            QueuedCommand::InjectUserMessage(text) => {
                messages.push(Message::user(text));
            }
            QueuedCommand::InjectSystemMessage(text) => {
                // System messages are injected as user turns with a [System]
                // prefix so they remain compatible with the Anthropic Messages API.
                messages.push(Message::user(format!("[System]: {text}")));
            }
            QueuedCommand::SetModel(_)
            | QueuedCommand::Compact
            | QueuedCommand::Clear
            | QueuedCommand::TriggerSkill(_) => {
                // Handled at the TUI/app layer; the query loop ignores them.
            }
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_priority_commands_are_fifo() {
        let queue = CommandQueue::new();
        for index in 0..8 {
            queue.push(
                QueuedCommand::InjectUserMessage(index.to_string()),
                CommandPriority::Normal,
            );
        }
        let actual: Vec<_> = queue
            .drain()
            .into_iter()
            .map(|command| match command {
                QueuedCommand::InjectUserMessage(text) => text,
                other => panic!("unexpected command: {other:?}"),
            })
            .collect();
        assert_eq!(
            actual,
            (0..8).map(|index| index.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn priority_preempts_fifo_across_cloned_handles() {
        let queue = CommandQueue::default();
        let cloned = queue.clone();
        assert!(queue.is_empty());
        for priority in [
            CommandPriority::Low,
            CommandPriority::Normal,
            CommandPriority::High,
            CommandPriority::Interrupt,
        ] {
            queue.push(
                QueuedCommand::InjectUserMessage(format!("{priority:?}-first")),
                priority,
            );
            cloned.push(
                QueuedCommand::InjectUserMessage(format!("{priority:?}-second")),
                priority,
            );
        }
        assert!(!cloned.is_empty());
        let actual: Vec<_> = cloned
            .drain()
            .into_iter()
            .map(|command| match command {
                QueuedCommand::InjectUserMessage(text) => text,
                other => panic!("unexpected command: {other:?}"),
            })
            .collect();
        assert_eq!(
            actual,
            [
                "Interrupt-first",
                "Interrupt-second",
                "High-first",
                "High-second",
                "Normal-first",
                "Normal-second",
                "Low-first",
                "Low-second"
            ]
        );
        assert!(queue.is_empty());
        assert!(queue.drain().is_empty());
        cloned.push(QueuedCommand::Clear, CommandPriority::Low);
        assert!(matches!(queue.drain().as_slice(), [QueuedCommand::Clear]));
        assert!(cloned.is_empty());
    }

    #[test]
    fn drain_preserves_message_mapping_and_ui_command_policy() {
        use claurst_core::types::{MessageContent, Role};
        let queue = CommandQueue::new();
        for command in [
            QueuedCommand::Compact,
            QueuedCommand::Clear,
            QueuedCommand::SetModel("model".into()),
            QueuedCommand::TriggerSkill("skill".into()),
            QueuedCommand::InjectUserMessage("plain".into()),
            QueuedCommand::InjectSystemMessage("notice".into()),
        ] {
            queue.push(command, CommandPriority::Normal);
        }
        let messages = drain_command_queue(&queue);
        assert_eq!(messages.len(), 2);
        for (message, expected) in messages.iter().zip(["plain", "[System]: notice"]) {
            assert!(matches!(message.role, Role::User));
            assert!(matches!(&message.content, MessageContent::Text(text) if text == expected));
        }
        assert!(queue.is_empty());
    }
}
