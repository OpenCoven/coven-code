//! `/handoff` command support.
//!
//! The TUI builds a compact handoff packet from recent conversation context and
//! asks the Coven daemon to open a new session for the requested familiar.

use claurst_core::coven_shared::{CreateSessionRequest, DaemonClient};
use claurst_core::types::{Message, Role};

/// Format the last messages as a markdown context block suitable for handoff.
pub fn build_handoff_context(messages: &[Message], familiar_name: &str) -> String {
    let conversation = messages
        .iter()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(format_message)
        .collect::<Vec<_>>()
        .join("\n");

    let topic = messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| truncate_chars(&m.get_all_text(), 120))
        .unwrap_or_else(|| "(unknown topic)".to_string());

    format!(
        "# Handoff Context\n\
         **From:** coven-code session\n\
         **Familiar:** {familiar_name}\n\
         \n\
         ## Recent conversation\n\
         \n\
         {conversation}\n\
         ## Handoff request\n\
         Continue this work as {familiar_name}. The user was working on: {topic}.\n"
    )
}

/// Create a Coven daemon session for a named familiar.
pub fn send_handoff(
    familiar_name: &str,
    context: String,
    project_root: &str,
) -> Result<String, String> {
    let client = DaemonClient::new().ok_or_else(|| {
        let detection = claurst_core::openclaw_agent::detect_openclaw_agent_installation();
        format_handoff_unavailable_message(&detection)
    })?;
    let title = format!("Handoff from coven-code: {}", infer_short_topic(&context));
    client
        .create_session(CreateSessionRequest {
            familiar: familiar_name.to_string(),
            project_root: project_root.to_string(),
            harness: "openclaw".to_string(),
            title,
            initial_message: context,
        })
        .map_err(|e| e.to_string())
}

fn format_handoff_unavailable_message(
    detection: &claurst_core::openclaw_agent::OpenClawAgentDetection,
) -> String {
    let mut message =
        "Coven daemon not running. Start it with `coven daemon start` to use /handoff.".to_string();
    if detection.branch != claurst_core::openclaw_agent::OpenClawAgentDetectionBranch::NoOpenClawDir
    {
        message.push_str("\n\n");
        message.push_str(&detection.guidance);
    }
    message
}

fn format_message(message: &Message) -> String {
    let role = match message.role {
        Role::User => "User",
        Role::Assistant => "Assistant",
    };
    let text = truncate_chars(&message.get_all_text(), 500);
    text.lines()
        .map(|line| format!("> **{role}**: {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn infer_short_topic(context: &str) -> String {
    let Some((_, topic)) = context.split_once("The user was working on: ") else {
        return "session context".to_string();
    };
    let topic = topic.lines().next().unwrap_or("").trim_end_matches('.');
    if topic.is_empty() {
        "session context".to_string()
    } else {
        truncate_chars(topic, 60)
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_context_handles_empty_messages() {
        let ctx = build_handoff_context(&[], "wisp");
        assert!(ctx.contains("**Familiar:** wisp"));
        assert!(ctx.contains("Continue this work as wisp"));
        assert!(ctx.contains("(unknown topic)"));
    }

    #[test]
    fn handoff_context_includes_recent_messages_and_topic() {
        let msgs = vec![
            Message::user("Fix the login bug"),
            Message::assistant("I'll inspect auth"),
        ];
        let ctx = build_handoff_context(&msgs, "onyx");
        assert!(ctx.contains("> **User**: Fix the login bug"));
        assert!(ctx.contains("> **Assistant**: I'll inspect auth"));
        assert!(ctx.contains("The user was working on: Fix the login bug"));
    }

    #[test]
    fn truncate_chars_does_not_split_unicode_boundaries() {
        let text = "wisp 🌿 keeps context";
        assert_eq!(truncate_chars(text, 6), "wisp 🌿...");
    }

    #[test]
    fn unavailable_handoff_message_distinguishes_stale_openclaw_data() {
        let detection = claurst_core::openclaw_agent::OpenClawAgentDetection {
            branch: claurst_core::openclaw_agent::OpenClawAgentDetectionBranch::StaleOpenClawData,
            home: std::path::PathBuf::from("/tmp/.openclaw"),
            valid_agents: Vec::new(),
            stale_agents: Vec::new(),
            guidance: "OpenClaw data found, but no loadable OpenClaw agent was found. Create or import an agent; existing data will be preserved.".to_string(),
        };

        let message = format_handoff_unavailable_message(&detection);

        assert!(message.contains("OpenClaw data found"));
        assert!(message.contains("no loadable OpenClaw agent"));
        assert!(message.contains("Create or import"));
    }
}
