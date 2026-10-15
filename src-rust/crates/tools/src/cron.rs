// Cron tools: schedule recurring and one-shot prompts.
//
// CronCreateTool  – create a new scheduled task (cron expression)
// CronDeleteTool  – remove an existing scheduled task
// CronListTool    – list all scheduled tasks
//
// Scheduled tasks are stored in a global in-memory store.
// Durable tasks are persisted to `~/.coven-code/scheduled_tasks.json`.
//
// On first use the store is initialised from the JSON file; tasks older than
// 7 days are automatically purged on load (matching TypeScript behaviour).
//
// Cron expression format: "M H DoM Mon DoW" (standard 5-field cron in local
// time). For example:
//   "*/5 * * * *"   = every 5 minutes
//   "30 14 * * 1"   = every Monday at 14:30

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Local, Timelike};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronTask {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub recurring: bool,
    pub durable: bool,
    pub created_at: u64,
}

/// 7 days in seconds — tasks older than this are purged on load.
const MAX_TASK_AGE_SECS: u64 = 7 * 24 * 3600;

/// Whether the store has been initialised from disk for this process.
static STORE_INITIALISED: once_cell::sync::Lazy<tokio::sync::Mutex<bool>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(false));

static CRON_STORE: Lazy<Arc<RwLock<HashMap<String, CronTask>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

// ---------------------------------------------------------------------------
// Disk path helpers
// ---------------------------------------------------------------------------

/// Path to `config_home()/scheduled_tasks.json` (legacy `~/.coven-code/`, or
/// `~/.coven/code/` under the unified coven CLI).
fn scheduled_tasks_path() -> PathBuf {
    claurst_core::config::config_home().join("scheduled_tasks.json")
}

/// Ensure the store has been loaded from disk (once per process).
async fn ensure_store_loaded() -> Result<(), String> {
    let mut init = STORE_INITIALISED.lock().await;
    if *init {
        return Ok(());
    }

    // Load from ~/.coven-code/scheduled_tasks.json if it exists.
    let path = scheduled_tasks_path();

    let data = match tokio::fs::read_to_string(&path).await {
        Ok(d) => d,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            *init = true;
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "Failed to read cron tasks from {}: {}",
                path.display(),
                error
            ))
        }
    };

    let tasks: Vec<CronTask> = serde_json::from_str(&data).map_err(|error| {
        format!(
            "Failed to parse cron tasks from {}: {}",
            path.display(),
            error
        )
    })?;

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut store = CRON_STORE.write().await;
    for task in tasks {
        // Drop tasks older than MAX_TASK_AGE_SECS
        if now_secs.saturating_sub(task.created_at) > MAX_TASK_AGE_SECS {
            debug!("Cron task {} expired, skipping on load", task.id);
            continue;
        }
        store.insert(task.id.clone(), task);
    }
    *init = true;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public scheduler API (used by cc-query cron_scheduler)
// ---------------------------------------------------------------------------

/// Check if a cron expression fires at the given minute-resolution datetime.
pub fn cron_matches(expr: &str, dt: &DateTime<Local>) -> bool {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }
    let minute = dt.minute();
    let hour = dt.hour();
    let day = dt.day();
    let month = dt.month();
    let dow = dt.weekday().num_days_from_sunday(); // 0=Sun .. 6=Sat

    cron_field_matches(fields[0], minute, false)
        && cron_field_matches(fields[1], hour, false)
        && cron_field_matches(fields[2], day, false)
        && cron_field_matches(fields[3], month, false)
        && cron_field_matches(fields[4], dow, true)
}

fn cron_field_matches(field: &str, value: u32, sunday_alias: bool) -> bool {
    if field == "*" {
        return true;
    }
    // */N step
    if let Some(step_str) = field.strip_prefix("*/") {
        if let Ok(step) = step_str.parse::<u32>() {
            return step > 0 && value.is_multiple_of(step);
        }
    }
    // Comma-separated list of values or ranges
    for part in field.split(',') {
        if cron_range_matches(part, value, sunday_alias) {
            return true;
        }
    }
    false
}

fn cron_range_matches(part: &str, value: u32, sunday_alias: bool) -> bool {
    if let Some(dash) = part.find('-') {
        let lo: u32 = part[..dash].parse().unwrap_or(u32::MAX);
        let hi: u32 = part[dash + 1..].parse().unwrap_or(0);
        value >= lo && value <= hi
    } else {
        // 7 is a Sunday alias only in the day-of-week field.
        part.parse::<u32>()
            .is_ok_and(|n| n == value || (sunday_alias && n == 7 && value == 0))
    }
}

/// Return all tasks whose cron expression fires at `dt`.
/// One-shot tasks (recurring=false) are removed before being returned.
/// Durable one-shots are withheld if their removal cannot be persisted.
/// Persistence precedes delivery, not execution: a crash between them can lose a run.
pub async fn pop_due_tasks(dt: &DateTime<Local>) -> Vec<CronTask> {
    if let Err(error) = ensure_store_loaded().await {
        warn!(%error, "Cannot load scheduled tasks; skipping this scheduler tick");
        return Vec::new();
    }
    let mut store = CRON_STORE.write().await;
    let mut due: Vec<CronTask> = store
        .values()
        .filter(|t| cron_matches(&t.cron, dt))
        .cloned()
        .collect();
    if due.iter().any(|task| task.durable && !task.recurring) {
        let remaining = store.values().filter(|task| {
            !due.iter()
                .any(|due| due.durable && !due.recurring && due.id == task.id)
        });
        if let Err(error) = persist_tasks_to_disk(remaining).await {
            warn!(
                path = %scheduled_tasks_path().display(),
                %error,
                "Failed to persist durable one-shot removal; withholding those tasks"
            );
            due.retain(|task| task.recurring || !task.durable);
        }
    }
    for t in &due {
        if !t.recurring {
            store.remove(&t.id);
        }
    }
    due
}

// ---------------------------------------------------------------------------
// Simple cron expression parser (5-field)
// ---------------------------------------------------------------------------

/// Validate that a 5-field cron expression is syntactically correct.
fn validate_cron(expr: &str) -> bool {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }
    // Check each field: ranges for M(0-59), H(0-23), DoM(1-31), Mon(1-12), DoW(0-7)
    let ranges = [(0u32, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
    for (i, field) in fields.iter().enumerate() {
        if *field == "*" {
            continue;
        }
        // Handle */N (step)
        if let Some(step) = field.strip_prefix("*/") {
            if !step.parse::<u32>().is_ok_and(|step| step > 0) {
                return false;
            }
            continue;
        }
        // Handle N-M (range) or N
        let (lo, hi) = field.split_once('-').unwrap_or((field, field));
        match (lo.parse::<u32>(), hi.parse::<u32>()) {
            (Ok(lo), Ok(hi)) if ranges[i].0 <= lo && lo <= hi && hi <= ranges[i].1 => {}
            _ => return false,
        }
    }
    true
}

/// Convert a cron expression to a human-readable description.
fn cron_to_human(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return expr.to_string();
    }

    let (minute, hour, dom, month, dow) = (fields[0], fields[1], fields[2], fields[3], fields[4]);

    if expr == "* * * * *" {
        return "every minute".to_string();
    }
    if let Some(n) = minute.strip_prefix("*/") {
        return format!("every {} minutes", n);
    }
    if hour == "*" && dom == "*" && month == "*" && dow == "*" {
        return format!("at minute {} of every hour", minute);
    }
    if dom == "*" && month == "*" && dow == "*" {
        return format!("daily at {:0>2}:{:0>2}", hour, minute);
    }
    // Fallback: return the raw expression
    format!("cron({})", expr)
}

// ---------------------------------------------------------------------------
// CronCreate
// ---------------------------------------------------------------------------

pub struct CronCreateTool;

#[derive(Debug, Deserialize)]
struct CronCreateInput {
    cron: String,
    prompt: String,
    #[serde(default = "default_true")]
    recurring: bool,
    #[serde(default)]
    durable: bool,
}

fn default_true() -> bool {
    true
}

#[async_trait]
impl Tool for CronCreateTool {
    fn name(&self) -> &str {
        "CronCreate"
    }

    fn description(&self) -> &str {
        "Schedule a recurring or one-shot prompt using a standard 5-field cron expression \
         in local time: \"M H DoM Mon DoW\". Examples:\n\
         - \"*/5 * * * *\" = every 5 minutes\n\
         - \"30 14 * * 1\" = every Monday at 14:30\n\
         - \"0 9 15 * *\" = 15th of each month at 09:00\n\
         Use recurring=false for one-shot (fires once then auto-deletes).\n\
         Use durable=true to persist across sessions."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "cron": {
                    "type": "string",
                    "description": "5-field cron expression: M H DoM Mon DoW"
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt to run at each scheduled time"
                },
                "recurring": {
                    "type": "boolean",
                    "description": "true (default) = repeat on every match; false = fire once then delete"
                },
                "durable": {
                    "type": "boolean",
                    "description": "true = persist to .coven-code/scheduled_tasks.json; false (default) = session only"
                }
            },
            "required": ["cron", "prompt"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> ToolResult {
        let params: CronCreateInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        if !validate_cron(&params.cron) {
            return ToolResult::error(format!(
                "Invalid cron expression '{}'. Expected 5 fields: M H DoM Mon DoW.",
                params.cron
            ));
        }

        // Ensure persistent tasks are loaded from disk before we check the count.
        if let Err(error) = ensure_store_loaded().await {
            return ToolResult::error(error);
        }

        let mut store = CRON_STORE.write().await;
        if store.len() >= 50 {
            return ToolResult::error(
                "Too many scheduled jobs (max 50). Cancel one first.".to_string(),
            );
        }

        let id = Uuid::new_v4().to_string()[..8].to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let task = CronTask {
            id: id.clone(),
            cron: params.cron.clone(),
            prompt: params.prompt.clone(),
            recurring: params.recurring,
            durable: params.durable,
            created_at: now,
        };

        let previous = store.insert(id.clone(), task);

        // Persist to ~/.coven-code/scheduled_tasks.json for durable tasks.
        if params.durable {
            if let Err(e) = persist_tasks_to_disk(store.values()).await {
                if let Some(previous) = previous {
                    store.insert(id, previous);
                } else {
                    store.remove(&id);
                }
                return ToolResult::error(format!(
                    "Failed to create durable cron task: could not write {}: {}",
                    scheduled_tasks_path().display(),
                    e
                ));
            }
        }

        let human = cron_to_human(&params.cron);

        let where_note = if params.durable {
            "Persisted to ~/.coven-code/scheduled_tasks.json"
        } else {
            "Session-only (dies when Claude exits)"
        };

        let msg = if params.recurring {
            format!("Scheduled recurring job {} ({}). {}", id, human, where_note)
        } else {
            format!(
                "Scheduled one-shot task {} ({}). {}. Will fire once then auto-delete.",
                id, human, where_note
            )
        };

        ToolResult::success(msg)
    }
}

// ---------------------------------------------------------------------------
// CronDelete
// ---------------------------------------------------------------------------

pub struct CronDeleteTool;

#[derive(Debug, Deserialize)]
struct CronDeleteInput {
    id: String,
}

#[async_trait]
impl Tool for CronDeleteTool {
    fn name(&self) -> &str {
        "CronDelete"
    }

    fn description(&self) -> &str {
        "Cancel a scheduled cron task by its ID. Use CronList to find the ID."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The cron task ID to delete"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> ToolResult {
        let params: CronDeleteInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        // Load disk state first so we don't accidentally drop persisted tasks.
        if let Err(error) = ensure_store_loaded().await {
            return ToolResult::error(error);
        }

        let mut store = CRON_STORE.write().await;
        if let Some(removed) = store.remove(&params.id) {
            // If it was durable, update the file.
            if removed.durable {
                if let Err(e) = persist_tasks_to_disk(store.values()).await {
                    store.insert(params.id, removed);
                    return ToolResult::error(format!(
                        "Failed to delete durable cron task: could not write {}: {}",
                        scheduled_tasks_path().display(),
                        e
                    ));
                }
            }
            ToolResult::success(format!("Deleted cron task '{}'.", params.id))
        } else {
            ToolResult::error(format!("Cron task '{}' not found.", params.id))
        }
    }
}

// ---------------------------------------------------------------------------
// CronList
// ---------------------------------------------------------------------------

pub struct CronListTool;

#[async_trait]
impl Tool for CronListTool {
    fn name(&self) -> &str {
        "CronList"
    }

    fn description(&self) -> &str {
        "List all currently scheduled cron tasks."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> ToolResult {
        // Merge in-memory store with any persisted tasks from disk.
        if let Err(error) = ensure_store_loaded().await {
            return ToolResult::error(error);
        }

        let store = CRON_STORE.read().await;

        if store.is_empty() {
            return ToolResult::success("No scheduled cron tasks.".to_string());
        }

        let mut tasks: Vec<&CronTask> = store.values().collect();
        tasks.sort_by_key(|t| t.created_at);

        let lines: Vec<String> = tasks
            .iter()
            .map(|t| {
                format!(
                    "{} | {} | {} | recurring={} | durable={} | prompt: {}",
                    t.id,
                    t.cron,
                    cron_to_human(&t.cron),
                    t.recurring,
                    t.durable,
                    if t.prompt.len() > 60 {
                        let mut end = 60;
                        while !t.prompt.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}…", &t.prompt[..end])
                    } else {
                        t.prompt.clone()
                    }
                )
            })
            .collect();

        ToolResult::success(format!(
            "Scheduled tasks ({}):\n\n{}",
            tasks.len(),
            lines.join("\n")
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Atomically replace the durable snapshot using a same-directory private file.
/// Destination symlinks are replaced; a symlinked engine home remains supported.
/// File contents are synced before replacement, but the parent directory is not.
async fn persist_tasks_to_disk<'a>(
    tasks: impl Iterator<Item = &'a CronTask>,
) -> Result<(), String> {
    let durable: Vec<&CronTask> = tasks.filter(|t| t.durable).collect();
    let json = serde_json::to_string_pretty(&durable).map_err(|e| e.to_string())?;

    let path = scheduled_tasks_path();
    let dir = path.parent().ok_or("No parent directory")?;

    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| e.to_string())?;

    let dir = dir.to_path_buf();
    let staged =
        tokio::task::spawn_blocking(move || -> std::io::Result<tempfile::NamedTempFile> {
            use std::io::Write;
            let mut staged = tempfile::NamedTempFile::new_in(dir)?;
            staged.write_all(json.as_bytes())?;
            staged.as_file().sync_all()?;
            Ok(staged)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    // Publish while the caller still holds the store lock. A cancelled staging
    // future must not leave a detached blocking task that can publish stale state.
    staged.persist(&path).map_err(|e| e.error.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn literal_seven_is_not_zero_in_minute_or_hour() {
        let midnight = Local
            .with_ymd_and_hms(2026, 1, 4, 0, 0, 0)
            .single()
            .unwrap();
        let seven = Local
            .with_ymd_and_hms(2026, 1, 4, 7, 7, 0)
            .single()
            .unwrap();
        assert!(!cron_matches("7 * * * *", &midnight));
        assert!(!cron_matches("* 7 * * *", &midnight));
        assert!(cron_matches("7 * * * *", &seven));
        assert!(cron_matches("* 7 * * *", &seven));
    }

    #[test]
    fn sunday_alias_is_retained_in_day_of_week_literals() {
        let sunday = Local
            .with_ymd_and_hms(2026, 1, 4, 12, 0, 0)
            .single()
            .unwrap();
        let monday = Local
            .with_ymd_and_hms(2026, 1, 5, 12, 0, 0)
            .single()
            .unwrap();
        for expression in ["* * * * 0", "* * * * 7", "* * * * 0,7"] {
            assert!(cron_matches(expression, &sunday), "{expression}");
            assert!(!cron_matches(expression, &monday), "{expression}");
        }
        assert!(cron_matches("* * * * 1", &monday));
    }

    #[test]
    fn existing_wildcard_step_list_and_range_matching_is_preserved() {
        let date = Local
            .with_ymd_and_hms(2026, 1, 4, 12, 10, 0)
            .single()
            .unwrap();
        for expression in [
            "* * * * *",
            "*/5 * * * *",
            "9,10 * * * *",
            "10-12 * * * *",
            "* * * * 0-6",
        ] {
            assert!(cron_matches(expression, &date), "{expression}");
        }
        for expression in [
            "*/3 * * * *",
            "*/0 * * * *",
            "8,9 * * * *",
            "11-12 * * * *",
            "* * * * 6-7",
            "* * * *",
        ] {
            assert!(!cron_matches(expression, &date), "{expression}");
        }
    }
}

#[cfg(test)]
mod validation_tests {
    use super::validate_cron;

    #[test]
    fn rejects_zero_steps_in_every_field() {
        for index in 0..5 {
            let mut fields = ["*"; 5];
            fields[index] = "*/0";
            assert!(!validate_cron(&fields.join(" ")), "field {index}");
        }
    }

    #[test]
    fn rejects_reversed_and_multiple_ranges_in_every_field() {
        for index in 0..5 {
            for value in ["2-1", "1-2-3", "-1", "1-", "1--2"] {
                let mut fields = ["*"; 5];
                fields[index] = value;
                assert!(!validate_cron(&fields.join(" ")), "field {index}: {value}");
            }
        }
    }

    #[test]
    fn preserves_field_bounds_and_existing_grammar() {
        for (index, (min, max)) in [(0u32, 59u32), (0, 23), (1, 31), (1, 12), (0, 7)]
            .into_iter()
            .enumerate()
        {
            for value in [
                min.to_string(),
                max.to_string(),
                format!("{min}-{max}"),
                format!("{min}-{min}"),
                "*/1".into(),
                format!("*/{}", u32::MAX),
            ] {
                let mut fields = ["*"; 5];
                fields[index] = &value;
                assert!(validate_cron(&fields.join(" ")), "field {index}: {value}");
            }
            let mut invalid = vec![
                format!("{}", max + 1),
                format!("{min}-{}", max + 1),
                "1,2".into(),
                "*/x".into(),
                "*/4294967296".into(),
            ];
            if min > 0 {
                invalid.extend(["0".into(), format!("0-{max}")]);
            }
            for value in invalid {
                let mut fields = ["*"; 5];
                fields[index] = &value;
                assert!(!validate_cron(&fields.join(" ")), "field {index}: {value}");
            }
        }
        assert!(validate_cron("* * * * *"));
        assert!(!validate_cron("* * * *"));
        assert!(!validate_cron("* * * * * *"));
    }
}

#[cfg(test)]
mod load_tests {
    use super::*;
    use claurst_core::config::{Config, PermissionMode};
    use claurst_core::permissions::AutoPermissionHandler;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    #[test]
    fn cancelled_initialization_can_retry() {
        let home = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cron::load_tests::cancelled_load_child",
                "--ignored",
                "--nocapture",
            ])
            .env("COVEN_CODE_HOME", home.path())
            .env_remove("COVEN_CODE_TEST_HOME")
            .env("COVEN_CRON_LOAD_CASE", "cancelled")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "fixture timed out: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("cron-cancelled-load-complete"));
    }

    #[tokio::test]
    #[ignore = "run by isolated parent with temporary engine home"]
    async fn cancelled_load_child() {
        assert_eq!(std::env::var("COVEN_CRON_LOAD_CASE").unwrap(), "cancelled");
        let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
        let ctx = ToolContext {
            working_dir: home.clone(),
            permission_mode: PermissionMode::Default,
            permission_handler: Arc::new(AutoPermissionHandler {
                mode: PermissionMode::Default,
            }),
            cost_tracker: claurst_core::cost::CostTracker::new(),
            session_id: "cron-load-fixture".into(),
            goal_turn: Default::default(),
            file_history: Arc::new(parking_lot::Mutex::new(
                claurst_core::file_history::FileHistory::new(),
            )),
            current_turn: Arc::new(AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: None,
            config: Config::default(),
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
        };

        let task = CronTask {
            id: "retained".into(),
            cron: "* * * * *".into(),
            prompt: "stored".into(),
            recurring: true,
            durable: true,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };
        let stored = serde_json::to_vec(&vec![task]).unwrap();
        let path = home.join("scheduled_tasks.json");
        std::fs::write(&path, &stored).unwrap();
        let store_guard = CRON_STORE.write().await;
        {
            let load = ensure_store_loaded();
            tokio::pin!(load);
            assert!(futures::poll!(&mut load).is_pending());
            assert!(
                STORE_INITIALISED.try_lock().is_err(),
                "loader must have entered"
            );
        }
        drop(store_guard);
        let listed = CronListTool.execute(serde_json::json!({}), &ctx).await;
        assert!(!listed.is_error, "{}", listed.content);
        assert!(listed.content.contains("retained"), "{}", listed.content);
        assert_eq!(std::fs::read(path).unwrap(), stored);
        println!("cron-cancelled-load-complete");
    }
}
