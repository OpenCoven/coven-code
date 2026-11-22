//! Continue a single existing Claude research task through the provider's local
//! session. The parent may call SendMessage once, to the recorded child only.
use std::{path::Path, process::Stdio, time::Duration};

use anyhow::{bail, ensure, Context, Result};
use claurst_core::config::PermissionMode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
};
use uuid::Uuid;

use super::{
    build_args, cli_permission_mode, resolve_claude_binary, task_events::TaskEvents,
    READ_ONLY_CLI_MODE,
};
use crate::{ProviderTask, ProviderTaskPhase, StreamEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCheckpoint {
    pub provider_session_id: String,
    pub task_id: String,
    pub run_id: String,
}

impl TaskCheckpoint {
    fn valid(&self) -> bool {
        Uuid::parse_str(&self.provider_session_id)
            .is_ok_and(|id| id.to_string() == self.provider_session_id)
            && self.task_id.starts_with('a')
            && (8..=96).contains(&self.task_id.len())
            && self
                .task_id
                .bytes()
                .skip(1)
                .all(|b| b.is_ascii_hexdigit() || b == b'-')
            && !self.run_id.is_empty()
            && self.run_id.len() <= 256
            && self
                .run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    }
}

const GUARD: &str = "coven-recover-one-task";
const CONTINUE: &str =
    "Continue your unfinished assignment from its saved progress and return the remaining result.";
const READ_TOOLS: &[&str] = &["Read", "Glob", "Grep", "WebFetch", "WebSearch"];
const UNCONFIRMED: &str = "The provider did not confirm completion. Review the saved progress before trying another action.";

async fn write_frame(stdin: &mut ChildStdin, frame: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&frame)?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).await?;
    stdin.flush().await?;
    Ok(())
}

struct Dispatch<'a> {
    owner: &'a str,
    target: &'a str,
    call: Option<String>,
}

impl Dispatch<'_> {
    fn decide(&mut self, request: &Value) -> Value {
        let hook = &request["input"];
        if request["callback_id"] == GUARD
            && hook["hook_event_name"] == "PreToolUse"
            && hook["session_id"] == self.owner
        {
            if hook["agent_id"] == self.target
                && hook["tool_name"]
                    .as_str()
                    .is_some_and(|s| READ_TOOLS.contains(&s))
            {
                return json!({});
            }
            if self.call.is_none()
                && hook["agent_id"].is_null()
                && hook["tool_name"] == "SendMessage"
                && hook.pointer("/tool_input/to").and_then(Value::as_str) == Some(self.target)
            {
                if let Some(id) = hook["tool_use_id"].as_str().filter(|s| !s.is_empty()) {
                    self.call = Some(id.to_owned());
                    // Permission still follows the original policy. Keep the
                    // instruction fixed even if the dispatcher paraphrases it.
                    return json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{"to":self.target,"message":CONTINUE,"summary":"Continue unfinished research"}}});
                }
            }
        }
        json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Only the selected unfinished research task may continue."}})
    }
}

/// The narrow research recovery profile retains the normal policy and offers
/// only read and research tools. A terminal result from the parent alone cannot
/// confirm that the designated child actually continued.
pub async fn resume_research(
    target: &TaskCheckpoint,
    cwd: &Path,
    model: &str,
    permission: &PermissionMode,
    mut emit: impl FnMut(StreamEvent),
) -> Result<()> {
    ensure!(target.valid(), "The saved task checkpoint is invalid.");
    let owner = target.provider_session_id.to_string();
    let args = build_args(model, Some(&owner), Some(permission));
    let policy = super::super::claude_cli_policy::args_from_environment(
        cli_permission_mode(Some(permission)) == READ_ONLY_CLI_MODE,
    )
    .map_err(anyhow::Error::msg)?;
    let binary = resolve_claude_binary().context("Claude Code is unavailable on the host.")?;
    let mut child = Command::new(binary)
        .args(args)
        .args(policy)
        .args(["--tools", "SendMessage,Read,Glob,Grep,WebFetch,WebSearch"])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .context("Cannot initialize the provider.")?;
    let mut stdout = BufReader::new(
        child
            .stdout
            .take()
            .context("Cannot read the provider response.")?,
    );
    let init = Uuid::new_v4().to_string();
    write_frame(&mut stdin, json!({"type":"control_request","request_id":init,"request":{"subtype":"initialize","hooks":{"PreToolUse":[{"hookCallbackIds":[GUARD],"timeout":5,"onFailure":"block"}]}}})).await?;
    let mut sent = false;
    let mut tasks = TaskEvents::default();
    let mut dispatch = Dispatch {
        owner: &owner,
        target: &target.task_id,
        call: None,
    };
    let mut attempt = None;
    let mut delivered = false;
    let mut terminal = None;
    let mut resumable = false;
    loop {
        const MAX: u64 = 8 * 1024 * 1024;
        let mut line = String::new();
        let n = tokio::time::timeout(
            Duration::from_secs(if sent { 600 } else { 30 }),
            (&mut stdout).take(MAX + 1).read_line(&mut line),
        )
        .await
        .context(UNCONFIRMED)??;
        ensure!(n > 0 && n as u64 <= MAX, "{UNCONFIRMED}");
        let frame: Value = serde_json::from_str(&line)?;
        match frame["type"].as_str() {
            Some("control_response") if !sent && frame["response"]["request_id"] == init => {
                ensure!(
                    frame["response"]["subtype"] == "success",
                    "The provider could not initialize targeted recovery."
                );
                write_frame(&mut stdin, json!({"type":"user","message":{"role":"user","content":format!("Continue only the unfinished research task {}. Use SendMessage once with this exact recipient. Wait for its result.",target.task_id)},"parent_tool_use_id":null})).await?;
                sent = true;
            }
            Some("control_request") => {
                let request = &frame["request"];
                ensure!(request["subtype"]=="hook_callback", "The provider requires another permission. Review this task in your conversation.");
                write_frame(&mut stdin,json!({"type":"control_response","response":{"request_id":frame["request_id"],"subtype":"success","response":dispatch.decide(request)}})).await?;
            }
            _ if frame["session_id"] != owner => {
                bail!("The provider did not reopen the saved session.")
            }
            Some("system") => {
                let call = dispatch.call.as_deref();
                if let Some(task) = tasks.observe(&frame) {
                    ensure!(
                        task.task_id == target.task_id
                            && call.is_some()
                            && frame["tool_use_id"].as_str() == call,
                        "The provider reported an unexpected task."
                    );
                    match task.phase {
                        ProviderTaskPhase::Started => {
                            ensure!(
                                task.run_id.as_ref().is_some_and(|id| id != &target.run_id)
                                    && (attempt.is_none() || attempt == task.run_id),
                                "The provider did not open one new attempt."
                            );
                            attempt.clone_from(&task.run_id);
                        }
                        ProviderTaskPhase::Completed
                        | ProviderTaskPhase::Failed
                        | ProviderTaskPhase::Stopped
                            if attempt.is_some() && task.run_id == attempt =>
                        {
                            terminal = Some(task.phase == ProviderTaskPhase::Completed);
                        }
                        _ => {}
                    }
                    emit(StreamEvent::ProviderTask(task));
                }
            }
            Some("user")
                if frame["tool_use_result"]["success"] == true
                    && frame["tool_use_result"]["resumedAgentId"] == target.task_id =>
            {
                delivered |= frame
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| {
                        blocks.iter().any(|b| {
                            b["type"] == "tool_result"
                                && dispatch
                                    .call
                                    .as_ref()
                                    .is_some_and(|id| b["tool_use_id"].as_str() == Some(id))
                        })
                    });
            }
            Some("assistant" | "rate_limit_event") => {
                if let Some(limit) = tasks.limit(&frame) {
                    emit(StreamEvent::ProviderLimit(limit));
                } else if frame["agent_id"] == target.task_id && attempt.is_some() {
                    if let Some(blocks) =
                        frame.pointer("/message/content").and_then(Value::as_array)
                    {
                        for block in blocks.iter().filter(|b| b["type"] == "text") {
                            if let Some(text) = block["text"].as_str() {
                                emit(StreamEvent::TextDelta {
                                    index: 0,
                                    text: text.to_owned(),
                                });
                            }
                        }
                    }
                }
            }
            Some("result") if frame["is_error"] == true || (dispatch.call.is_none() && sent) => {
                bail!("{UNCONFIRMED}")
            }
            _ => {}
        }
        if delivered && !resumable {
            if let Some(run_id) = &attempt {
                emit(StreamEvent::ProviderTask(ProviderTask {
                    provider_session_id: owner.clone(),
                    task_id: target.task_id.clone(),
                    run_id: Some(run_id.clone()),
                    phase: ProviderTaskPhase::Continuable,
                    detail: String::new(),
                }));
                resumable = true;
            }
        }
        if resumable {
            match terminal {
                Some(true) => break,
                Some(false) => bail!("The continued researcher stopped before finishing. Review the progress in this conversation."),
                None => {}
            }
        }
    }
    drop(stdin);
    // Closing the control channel lets the provider flush the reopened session.
    // Bound shutdown even if its parent is waiting on a further model response.
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        child.kill().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(name: &str, caller: Option<&str>, recipient: &str) -> Value {
        json!({"callback_id":GUARD,"input":{"hook_event_name":"PreToolUse","session_id":"parent","agent_id":caller,"tool_name":name,"tool_use_id":"call","tool_input":{"to":recipient,"message":"unrelated instruction"}}})
    }

    #[test]
    fn only_the_saved_child_receives_one_fixed_recovery_instruction() {
        let mut guard = Dispatch {
            owner: "parent",
            target: "failed-child",
            call: None,
        };
        let denied = |r: &Value| r["hookSpecificOutput"]["permissionDecision"] == "deny";
        assert!(denied(&guard.decide(&hook(
            "SendMessage",
            None,
            "finished-child"
        ))));
        let mut wrong = hook("SendMessage", None, "failed-child");
        wrong["input"]["session_id"] = json!("different");
        assert!(denied(&guard.decide(&wrong)));
        assert!(guard.call.is_none());
        let permitted = guard.decide(&hook("SendMessage", None, "failed-child"));
        assert_eq!(
            permitted["hookSpecificOutput"]["updatedInput"]["message"],
            CONTINUE
        );
        assert_eq!(guard.call.as_deref(), Some("call"));
        assert!(denied(&guard.decide(&hook(
            "SendMessage",
            None,
            "failed-child"
        ))));
        assert_eq!(
            guard.decide(&hook("Read", Some("failed-child"), "")),
            json!({})
        );
        assert!(denied(&guard.decide(&hook(
            "Read",
            Some("finished-child"),
            ""
        ))));
        assert!(denied(&guard.decide(&hook(
            "Bash",
            Some("failed-child"),
            ""
        ))));
    }
}
