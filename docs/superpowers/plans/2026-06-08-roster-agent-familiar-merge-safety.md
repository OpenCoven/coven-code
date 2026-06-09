# Roster Agent Familiar Merge Safety Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make agents and familiars merge safely so `/plan` always resolves to the built-in read-only plan agent, saved familiars appear only when present in `~/.coven/familiars.toml`, and reset leaves no default familiar roster visible.

**Architecture:** Treat built-in agents and saved familiar roster entries as separate sources. The runtime merge keeps `build`, `plan`, and `explore` reserved, appends saved familiars from `~/.coven/familiars.toml`, and ignores stale/default familiar ids after reset. UI and slash-command surfaces must render from the same saved-roster contract instead of hardcoded familiar fallbacks.

**Tech Stack:** Rust, ratatui TUI, `claurst_core::coven_shared`, `claurst_tui`, command registry tests, cargo test/check.

---

## File Structure

- Modify: `src-rust/crates/core/src/coven_shared.rs`
  - Owns the built-in + config + saved-familiar agent merge and should contain precedence tests for reserved `/plan`.
- Modify: `src-rust/crates/tui/src/app.rs`
  - Owns runtime TUI state after `/agents reset`, F2 switcher population, and username familiar inference.
- Modify: `src-rust/crates/tui/src/render.rs`
  - Owns welcome/footer visibility for the selected familiar.
- Modify: `src-rust/crates/commands/src/lib.rs`
  - Owns `/familiar` slash-command roster display and switching.
- Modify: `src-rust/crates/commands/src/named_commands.rs`
  - Owns `coven-code agents reset` command behavior.
- Modify: `docs/familiars.md`
- Modify: `docs/src/content/familiars.js`

### Task 1: Core Merge Contract

**Files:**
- Modify: `src-rust/crates/core/src/coven_shared.rs`

- [ ] **Step 1: Write failing precedence tests**

Add these tests inside the existing `#[cfg(test)] mod tests` in `src-rust/crates/core/src/coven_shared.rs`:

```rust
#[test]
fn merge_keeps_plan_reserved_when_config_agent_collides() {
    let _guard = TestEnv::new();
    let mut config_agents = std::collections::HashMap::new();
    config_agents.insert(
        "plan".to_string(),
        crate::config::AgentDefinition {
            description: Some("unsafe shadow".to_string()),
            model: None,
            temperature: None,
            prompt: Some("shadow plan".to_string()),
            access: "full".to_string(),
            visible: true,
            max_turns: None,
            color: None,
        },
    );

    let merged = default_agents_with_familiars_and_config(&config_agents);
    let plan = merged.get("plan").expect("plan agent");

    assert_ne!(plan.prompt.as_deref(), Some("shadow plan"));
    assert_eq!(plan.access, "read-only");
}

#[test]
fn merge_keeps_plan_reserved_when_familiar_collides() {
    let guard = TestEnv::new();
    std::fs::create_dir_all(guard.coven_home()).expect("coven home");
    std::fs::write(
        guard.coven_home().join("familiars.toml"),
        "[[familiar]]\nid = \"plan\"\nrole = \"unsafe\"\naccess = \"full\"\n",
    )
    .expect("familiars");

    let merged = default_agents_with_familiars_and_config(&std::collections::HashMap::new());
    let plan = merged.get("plan").expect("plan agent");

    assert_eq!(plan.access, "read-only");
    assert!(!plan
        .description
        .as_deref()
        .unwrap_or_default()
        .contains("unsafe"));
}

#[test]
fn merge_includes_saved_non_reserved_familiars() {
    let guard = TestEnv::new();
    std::fs::create_dir_all(guard.coven_home()).expect("coven home");
    std::fs::write(
        guard.coven_home().join("familiars.toml"),
        "[[familiar]]\nid = \"sage\"\nrole = \"Research\"\naccess = \"read-only\"\n",
    )
    .expect("familiars");

    let merged = default_agents_with_familiars_and_config(&std::collections::HashMap::new());
    let sage = merged.get("sage").expect("saved familiar agent");

    assert_eq!(sage.access, "read-only");
    assert!(sage
        .prompt
        .as_deref()
        .unwrap_or_default()
        .contains("Research"));
}
```

If `TestEnv` does not currently expose the coven home path, add this method to the test helper:

```rust
impl TestEnv {
    fn coven_home(&self) -> &std::path::Path {
        &self.coven_home
    }
}
```

- [ ] **Step 2: Run tests to verify failure or current coverage**

Run:

```bash
cd src-rust
cargo test --package claurst-core merge_keeps_plan_reserved merge_includes_saved_non_reserved_familiars -- --nocapture
```

Expected before implementation: at least the familiar collision or saved familiar test should expose whether the current merge contract is incomplete. If all pass, keep the tests as regression coverage and do not change implementation in this task.

- [ ] **Step 3: Implement the smallest merge fix if needed**

Keep this intended shape in `default_agents_with_familiars_and_config`:

```rust
pub fn default_agents_with_familiars_and_config(
    config_agents: &std::collections::HashMap<String, crate::config::AgentDefinition>,
) -> std::collections::HashMap<String, crate::config::AgentDefinition> {
    let builtins = crate::config::default_agents();
    let mut map = builtins.clone();

    for (id, def) in config_agents {
        if !builtins.contains_key(id) {
            map.insert(id.clone(), def.clone());
        }
    }

    if let Some(fams) = load_familiars() {
        for fam in &fams {
            let (id, def) = familiar_to_agent_definition(fam);
            if !builtins.contains_key(&id) {
                map.insert(id, def);
            }
        }
    }

    map
}
```

- [ ] **Step 4: Re-run focused core tests**

Run:

```bash
cd src-rust
cargo test --package claurst-core merge_keeps_plan_reserved merge_includes_saved_non_reserved_familiars -- --nocapture
```

Expected: all added tests pass.

### Task 2: Saved Familiar Roster Only

**Files:**
- Modify: `src-rust/crates/tui/src/app.rs`
- Modify: `src-rust/crates/commands/src/lib.rs`

- [ ] **Step 1: Add failing app tests for no default familiar fallback**

Add these tests near the existing familiar switcher tests in `src-rust/crates/tui/src/app.rs`:

```rust
#[test]
fn app_does_not_infer_builtin_familiar_without_saved_roster() {
    let _lock = HOME_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let coven_home = temp.path().join("coven");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&coven_home).expect("coven home dir");
    let _guard = EnvGuard::set(&home, &coven_home);
    std::env::set_var("USER", "sage");

    let app = make_app();

    assert!(app.config.familiar.is_none());
    assert!(app.familiar_switcher_list.is_empty());
}

#[test]
fn app_infers_familiar_only_from_saved_roster() {
    let _lock = HOME_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let coven_home = temp.path().join("coven");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&coven_home).expect("coven home dir");
    std::fs::write(
        coven_home.join("familiars.toml"),
        "[[familiar]]\nid = \"sage\"\ndisplay_name = \"Sage\"\n",
    )
    .expect("familiars");
    let _guard = EnvGuard::set(&home, &coven_home);
    std::env::set_var("USER", "sage");

    let app = make_app();

    assert_eq!(app.config.familiar.as_deref(), Some("sage"));
    assert_eq!(app.familiar_switcher_list, vec!["sage".to_string()]);
}
```

- [ ] **Step 2: Add failing command tests for `/familiar` empty roster behavior**

Add command-level tests in `src-rust/crates/commands/src/lib.rs` beside existing `FamiliarCommand` tests, or create the smallest local test helper if no adjacent module exists:

```rust
#[tokio::test]
async fn familiar_command_without_roster_reports_none() {
    let _lock = HOME_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let coven_home = temp.path().join("coven");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&coven_home).expect("coven home dir");
    let _guard = EnvGuard::set(&home, &coven_home);

    let mut ctx = test_command_context();
    let result = FamiliarCommand.execute("", &mut ctx).await;

    assert!(matches!(result, CommandResult::Message(ref msg) if msg.contains("Current familiar: none")));
    assert!(matches!(result, CommandResult::Message(ref msg) if !msg.contains("kitty")));
}
```

- [ ] **Step 3: Run tests to verify failure**

Run:

```bash
cd src-rust
cargo test --package claurst-tui app_does_not_infer_builtin_familiar_without_saved_roster app_infers_familiar_only_from_saved_roster -- --nocapture
cargo test --package claurst-commands familiar_command_without_roster_reports_none -- --nocapture
```

Expected before implementation: app test fails because `infer_familiar_from_env` can infer built-in ids without a saved roster; command test fails because `/familiar` still falls back to the hardcoded familiar roster.

- [ ] **Step 4: Remove hardcoded familiar fallback from runtime roster logic**

Change `App::infer_familiar_from_env` in `src-rust/crates/tui/src/app.rs` so it only returns ids found in `coven_shared::load_familiars()`:

```rust
pub fn infer_familiar_from_env() -> Option<String> {
    use claurst_core::coven_shared;
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()?;
    let user_lc = user.to_lowercase();

    let familiars = coven_shared::load_familiars()?;
    for fam in &familiars {
        if user_lc.contains(&fam.id.to_lowercase()) {
            return Some(fam.id.clone());
        }
        if let Some(display_name) = &fam.display_name {
            if user_lc.contains(&display_name.to_lowercase()) {
                return Some(fam.id.clone());
            }
        }
    }

    None
}
```

Change `current_familiar_roster` in `src-rust/crates/commands/src/lib.rs` so it returns only daemon/saved roster entries:

```rust
fn current_familiar_roster() -> Vec<(String, String)> {
    claurst_core::coven_shared::load_familiars()
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            let desc = format_daemon_familiar(&f);
            (f.id, desc)
        })
        .collect()
}
```

Update `FamiliarCommand::execute` empty-args display to render `Current familiar: none` when `ctx.config.familiar` is unset or absent from the current roster:

```rust
let current = ctx
    .config
    .familiar
    .as_deref()
    .filter(|id| roster.iter().any(|(name, _)| name == id))
    .unwrap_or("none");
```

Update reset message:

```rust
None => "Familiar reset to none.".to_string(),
```

- [ ] **Step 5: Re-run focused tests**

Run:

```bash
cd src-rust
cargo test --package claurst-tui app_does_not_infer_builtin_familiar_without_saved_roster app_infers_familiar_only_from_saved_roster -- --nocapture
cargo test --package claurst-commands familiar_command_without_roster_reports_none -- --nocapture
```

Expected: all focused tests pass.

### Task 3: Reset Runtime State and Visibility

**Files:**
- Modify: `src-rust/crates/tui/src/app.rs`
- Modify: `src-rust/crates/tui/src/render.rs`
- Modify: `src-rust/crates/core/src/roster_reset.rs`
- Modify: `src-rust/crates/commands/src/named_commands.rs`
- Modify: `docs/familiars.md`
- Modify: `docs/src/content/familiars.js`

- [ ] **Step 1: Expand reset app regression test**

Replace `reset_agents_and_familiars_leaves_familiar_switcher_empty_without_roster` in `src-rust/crates/tui/src/app.rs` with:

```rust
#[test]
fn reset_agents_and_familiars_clears_runtime_roster_state() {
    let _lock = HOME_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let coven_home = temp.path().join("coven");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&coven_home).expect("coven home dir");
    std::fs::create_dir_all(&project).expect("project dir");
    std::fs::write(
        coven_home.join("familiars.toml"),
        "[[familiar]]\nid = \"sage\"\nemoji = \"🌿\"\n",
    )
    .expect("familiars");
    let _guard = EnvGuard::set(&home, &coven_home);

    let mut app = make_app();
    app.agents_menu.project_root = Some(project);
    app.config.familiar = Some("sage".to_string());
    app.config.agents.insert("custom".to_string(), claurst_core::config::AgentDefinition {
        description: Some("custom".to_string()),
        model: None,
        temperature: None,
        prompt: Some("custom".to_string()),
        access: "full".to_string(),
        visible: true,
        max_turns: None,
        color: None,
    });
    app.config.managed_agents = Some(claurst_core::config::ManagedAgentConfig::default());
    app.agent_mode = Some("sage".to_string());
    app.plan_mode = true;
    app.managed_agents_active = true;
    app.familiar_switcher_list = vec!["sage".to_string()];

    app.reset_agents_and_familiars();

    assert!(app.config.familiar.is_none());
    assert!(app.config.agents.is_empty());
    assert!(app.config.managed_agents.is_none());
    assert!(app.agent_mode.is_none());
    assert!(app.agent_mode_changed);
    assert!(!app.plan_mode);
    assert!(!app.managed_agents_active);
    assert!(app.familiar_switcher_list.is_empty());
    assert_eq!(app.familiar_switcher_idx, 0);
    assert!(!coven_home.join("familiars.toml").exists());
}
```

If `ManagedAgentConfig::default()` is unavailable, use the `test_managed_agents()` helper already used in `roster_reset.rs` and duplicate it locally in the app test module.

- [ ] **Step 2: Keep or add render visibility tests**

Ensure these tests exist in `src-rust/crates/tui/src/render.rs`:

```rust
#[test]
fn welcome_familiar_label_uses_none_by_default() {
    let _lock = HOME_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let coven_home = temp.path().join("coven");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&coven_home).expect("coven home dir");
    let _guard = EnvGuard::set(&home, &coven_home);

    let app = make_test_app_with_model_and_familiar(None, None, None, None);
    assert_eq!(welcome_familiar_label(&app), "Familiar: none");
}

#[test]
fn welcome_familiar_label_hides_stale_config_without_roster() {
    let _lock = HOME_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let coven_home = temp.path().join("coven");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&coven_home).expect("coven home dir");
    let _guard = EnvGuard::set(&home, &coven_home);

    let app = make_test_app_with_model_and_familiar(None, None, Some("sage"), None);
    assert_eq!(welcome_familiar_label(&app), "Familiar: none");
}

#[test]
fn welcome_familiar_label_reflects_saved_roster_config() {
    let _lock = HOME_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let coven_home = temp.path().join("coven");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&coven_home).expect("coven home dir");
    std::fs::write(
        coven_home.join("familiars.toml"),
        "[[familiar]]\nid = \"sage\"\nemoji = \"🌿\"\n",
    )
    .expect("familiar roster");
    let _guard = EnvGuard::set(&home, &coven_home);

    let app = make_test_app_with_model_and_familiar(None, None, Some("sage"), None);
    assert_eq!(welcome_familiar_label(&app), "Familiar: sage");
}
```

- [ ] **Step 3: Verify reset helper still clears persisted settings**

Run:

```bash
cd src-rust
cargo test --package claurst-core reset_removes_user_roster_state_without_touching_unrelated_files reset_reports_no_change_when_roster_state_is_absent -- --nocapture
```

Expected: both tests pass. Do not expand `reset_familiars_and_agents` beyond removing `.md` agent files, `~/.coven/familiars.toml`, and roster settings unless a test proves a missing path.

- [ ] **Step 4: Run TUI visibility/reset tests**

Run:

```bash
cd src-rust
cargo test --package claurst-tui reset_agents_and_familiars_clears_runtime_roster_state welcome_familiar_label -- --nocapture
```

Expected: reset leaves no familiar switcher entries, no selected familiar, no active familiar agent mode, and welcome labels use `Familiar: none` when roster is gone.

- [ ] **Step 5: Update docs to match the saved-roster contract**

In `docs/familiars.md`, ensure the reset section says:

```markdown
After reset, Coven Code shows `Familiar: none`, the F2 switcher stays empty,
and `/familiar` does not expose built-in familiar defaults until a new
`~/.coven/familiars.toml` exists.
```

In `docs/src/content/familiars.js`, ensure the standalone behavior list says:

```html
<li>The welcome panel shows <code>Familiar: none</code> until a saved roster familiar is selected.</li>
<li>The <code>/familiar</code> command and <kbd>F2</kbd> switcher use only <code>~/.coven/familiars.toml</code>.</li>
```

### Task 4: Full Verification

**Files:**
- No additional edits.

- [ ] **Step 1: Format**

Run:

```bash
cd src-rust
cargo fmt --all
```

Expected: exits 0.

- [ ] **Step 2: Focused regression suite**

Run:

```bash
cd src-rust
cargo test --package claurst-core merge_keeps_plan_reserved merge_includes_saved_non_reserved_familiars reset_removes_user_roster_state_without_touching_unrelated_files -- --nocapture
cargo test --package claurst-tui reset_agents_and_familiars_clears_runtime_roster_state welcome_familiar_label app_does_not_infer_builtin_familiar_without_saved_roster app_infers_familiar_only_from_saved_roster -- --nocapture
cargo test --package claurst-commands familiar_command_without_roster_reports_none -- --nocapture
```

Expected: all focused tests pass.

- [ ] **Step 3: Workspace compile gate**

Run:

```bash
cd src-rust
cargo check --workspace
```

Expected: exits 0 with no warnings/errors.

- [ ] **Step 4: Diff hygiene**

Run:

```bash
git diff --check
git status --short
```

Expected: no whitespace errors. Modified files should be limited to the files named in this plan plus any already-approved visibility docs/render changes.

- [ ] **Step 5: Commit only if Val asks**

Do not commit by default. If Val explicitly asks to commit, stage only the files changed for this work:

```bash
git add src-rust/crates/core/src/coven_shared.rs
git add src-rust/crates/tui/src/app.rs
git add src-rust/crates/tui/src/render.rs
git add src-rust/crates/commands/src/lib.rs
git add src-rust/crates/commands/src/named_commands.rs
git add docs/familiars.md
git add docs/src/content/familiars.js
git add docs/superpowers/plans/2026-06-08-roster-agent-familiar-merge-safety.md
git commit -m "fix(tui): keep familiar roster reset authoritative"
```

## Self-Review

- Spec coverage: the plan covers `/plan` merge safety, saved familiar roster behavior, reset runtime state, welcome/footer visibility, CLI slash-command behavior, docs, and verification.
- Placeholder scan: no TBD/TODO/fill-in placeholders remain. The only conditional branch is explicit: if an existing helper lacks a method or `ManagedAgentConfig::default()`, add the provided local replacement.
- Type consistency: all referenced production functions already exist: `default_agents_with_familiars_and_config`, `load_familiars`, `reset_agents_and_familiars`, `welcome_familiar_label`, `current_familiar_roster`, and `FamiliarCommand`.
