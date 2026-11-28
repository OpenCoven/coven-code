//! Read only Claude Code's structured background agent notifications. Task output
//! paths and prompts stay with the provider. A summary is not a result artifact.
use std::collections::{HashMap, HashSet};

use crate::{ProviderTask, ProviderTaskPhase};
use serde_json::Value;

#[derive(Default)]
pub(super) struct TaskEvents {
    owner: Option<String>,
    launches: HashMap<String, (String, Option<String>)>,
    attempts: HashSet<(String, Option<String>)>,
}

fn identifier(value: &Value) -> Option<&str> {
    value.as_str().filter(|id| {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    })
}

impl TaskEvents {
    pub(super) fn limit(&self, envelope: &Value) -> Option<crate::ProviderLimit> {
        let owner = identifier(&envelope["session_id"])?;
        if self.owner.as_deref() != Some(owner) {
            return None;
        }
        let (notice, resets_at) = match envelope["type"].as_str()? {
            "assistant" if envelope["error"] == "rate_limit" => {
                let blocks = envelope.pointer("/message/content")?.as_array()?;
                let text = blocks.iter().find(|b| b["type"] == "text")?["text"].as_str()?;
                (Some(text.chars().take(4096).collect()), None)
            }
            "rate_limit_event"
                if envelope.pointer("/rate_limit_info/status")?.as_str()? == "rejected" =>
            {
                (
                    None,
                    envelope
                        .pointer("/rate_limit_info/resetsAt")
                        .and_then(Value::as_i64),
                )
            }
            _ => return None,
        };
        Some(crate::ProviderLimit {
            provider_session_id: owner.to_owned(),
            notice,
            resets_at,
        })
    }

    pub(super) fn launch_receipt(&mut self, frame: &Value) -> Option<ProviderTask> {
        let owner = identifier(&frame["session_id"])?;
        if self.owner.as_deref() != Some(owner) || !frame["parent_tool_use_id"].is_null() {
            return None;
        }
        let receipt = &frame["tool_use_result"];
        if receipt["isAsync"] != true
            || receipt["status"] != "async_launched"
            || receipt["canContinueAgent"] != true
        {
            return None;
        }
        let blocks = frame.pointer("/message/content")?.as_array()?;
        let call = blocks
            .iter()
            .find(|b| b["type"] == "tool_result" && b["is_error"] != true)?;
        let key = self.launches.get(identifier(&call["tool_use_id"])?)?;
        if Some(key.0.as_str()) != identifier(&receipt["agentId"]) || !self.attempts.contains(key) {
            return None;
        }
        Some(ProviderTask {
            provider_session_id: owner.to_owned(),
            task_id: key.0.clone(),
            run_id: key.1.clone(),
            phase: ProviderTaskPhase::Continuable,
            detail: String::new(),
        })
    }

    pub(super) fn observe(&mut self, envelope: &Value) -> Option<ProviderTask> {
        let subtype = envelope["subtype"].as_str()?;
        let owner = identifier(&envelope["session_id"])?;
        if subtype == "init" && envelope["parent_tool_use_id"].is_null() {
            if self.owner.as_deref() != Some(owner) {
                self.attempts.clear();
                self.launches.clear();
                self.owner = Some(owner.to_owned());
            }
            return None;
        }
        if self.owner.as_deref() != Some(owner) || envelope["ambient"] == true {
            return None;
        }
        let task = identifier(&envelope["task_id"])?;
        let run = match envelope.get("run_id") {
            None | Some(Value::Null) => None,
            Some(v) => Some(identifier(v)?.to_owned()),
        };
        let key = (task.to_owned(), run.clone());
        let (phase, detail) = match subtype {
            "task_started"
                if envelope["task_type"] == "local_agent"
                    || (envelope["task_type"].is_null()
                        && envelope["subagent_type"].is_string()) =>
            {
                if let Some(call) = identifier(&envelope["tool_use_id"]) {
                    self.launches.insert(call.to_owned(), key.clone());
                }
                self.attempts.insert(key);
                (ProviderTaskPhase::Started, &envelope["description"])
            }
            "task_notification" if self.attempts.contains(&key) => {
                let phase = match envelope["status"].as_str()? {
                    "completed" => ProviderTaskPhase::Completed,
                    "failed" => ProviderTaskPhase::Failed,
                    "stopped" => ProviderTaskPhase::Stopped,
                    _ => return None,
                };
                self.attempts.remove(&key);
                (phase, &envelope["summary"])
            }
            _ => return None,
        };
        Some(ProviderTask {
            provider_session_id: owner.to_owned(),
            task_id: task.to_owned(),
            run_id: run,
            phase,
            detail: detail
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(1024)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_terminal_receipt_requires_the_observed_child_owner_and_attempt() {
        let mut tasks = TaskEvents::default();
        let start = json!({"type":"system","subtype":"task_started","session_id":"parent","task_type":"local_agent","task_id":"child","run_id":"one","description":"Inspect colors"});
        let end = json!({"type":"system","subtype":"task_notification","session_id":"parent","task_id":"child","run_id":"one","status":"completed","summary":"Contrast checked","output_file":"/private/result"});
        assert!(tasks.observe(&start).is_none());
        tasks.observe(&json!({"type":"system","subtype":"init","session_id":"parent"}));
        assert_eq!(
            tasks.observe(&start).unwrap().phase,
            ProviderTaskPhase::Started
        );
        for field in ["session_id", "task_id", "run_id"] {
            let mut other = end.clone();
            other[field] = json!("different");
            assert!(tasks.observe(&other).is_none());
        }
        let receipt = tasks.observe(&end).unwrap();
        assert_eq!(receipt.phase, ProviderTaskPhase::Completed);
        assert!(!serde_json::to_string(&receipt)
            .unwrap()
            .contains("/private/result"));
        assert!(tasks.observe(&end).is_none());
    }
}
