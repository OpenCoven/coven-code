# Issue #59 Remainder: Rewind Unification + Dead `ide` Removal

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the two bullets of issue #59 that PR #68 left unfinished: (1) actually unify `/undo`/`/revert`/`/rewind` into a single rewind surface, and (2) remove the dead `ide` command after verification showed its integration cannot work.

**Architecture:** `/rewind` (already the only visible rewind command after #68) gains argument routing: bare `/rewind` keeps the interactive conversation-rewind overlay; any arguments forward to the shadow-git `RevertCommand` paths (`list`, `diff [n]`, `last`, `<n>`, `<uuid>`), so file rollback stays discoverable through the one visible command. The `ide` named command, its slash adapter, and the now-orphaned `core::ide` module are deleted because no `coven-code.coven-code` extension exists anywhere (verified: VS Code marketplace, OpenCoven org repos), so the lockfile bridge it reads has no possible writer.

**Tech Stack:** Rust workspace at `src-rust/` (crates `claurst-commands`, `claurst-tui`, `claurst-core`). Tests: `cargo test -p claurst-commands`.

**Verification evidence for `ide` removal (bullet 4 of #59):**
- `IdeCommand` (`named_commands.rs`) suggests `code --install-extension coven-code.coven-code`; no such extension exists on the VS Code marketplace or Open VSX (web-checked 2026-06-10).
- OpenCoven publishes no extension repo (checked all 45 org repos).
- The `~/.coven-code/ide/*.lock` files it reads are written only by that nonexistent extension.
- Real IDE support is the separate ACP crate (`coven-code acp`), untouched by this change.
- `claurst_core::detect_ide` / `IdeKind::extension_install_command` have no consumers other than `IdeCommand` (verified by grep), so `core::ide` goes too.

---

### Task 1: Route `/rewind` arguments to the shadow-git revert paths

**Files:**
- Modify: `src-rust/crates/commands/src/lib.rs` (RewindCommand impl, ~line 5170; tests module ~line 9650+)
- Modify: `src-rust/crates/tui/src/app.rs` (PROMPT_SLASH_COMMANDS `rewind` entry, ~line 133)
- Modify: `docs/commands.md` (`### /rewind` section, ~line 126)

- [ ] **Step 1: Write the failing tests** (in `mod tests` in `src-rust/crates/commands/src/lib.rs`, next to `ide_slash_adapter_uses_named_command`)

```rust
#[tokio::test]
async fn rewind_with_args_routes_to_file_rollback_not_overlay() {
    let _guard = CommandEnvGuard::with_coven_home(None);
    let mut ctx = make_ctx();
    let command = find_command("rewind").unwrap();

    // `list` must reach the checkpoints path, never the overlay.
    let result = command.execute("list", &mut ctx).await;
    assert!(
        !matches!(result, CommandResult::OpenRewindOverlay),
        "/rewind list must route to the revert/checkpoints path"
    );

    // `diff` must reach the snapshot-diff path, never the overlay.
    let result = command.execute("diff", &mut ctx).await;
    assert!(
        !matches!(result, CommandResult::OpenRewindOverlay),
        "/rewind diff must route to the snapshot diff path"
    );
}

#[tokio::test]
async fn rewind_without_args_keeps_existing_behavior() {
    let mut ctx = make_ctx();
    let command = find_command("rewind").unwrap();
    let result = command.execute("", &mut ctx).await;
    match result {
        CommandResult::Message(message) => assert!(message.contains("Nothing to rewind")),
        other => panic!("expected empty-conversation message, got {:?}", other),
    }
}
```

- [ ] **Step 2: Run tests to verify the first one fails**

Run: `cd src-rust && cargo test -p claurst-commands rewind_with_args -- --nocapture`
Expected: FAIL — current `execute(&self, _args, …)` ignores args and returns `OpenRewindOverlay` (conversation empty → actually returns the "Nothing to rewind" message; the assertion that matters: with a message in ctx it returns overlay. If both pass trivially because `ctx.messages` is empty, the empty-conversation early-return fires before the overlay — adjust: push one message first):

```rust
ctx.messages.push(claurst_core::types::Message::user("hi"));
```

(Use whatever constructor existing tests/`claurst_core::types::Message` provide; check with `grep -n "fn user\|Message::user\|Message {" crates/core/src/types.rs`.)

- [ ] **Step 3: Implement the routing in `RewindCommand::execute`**

Replace the current impl body (keep struct/registration):

```rust
fn help(&self) -> &str {
    "Usage: /rewind [list|diff [n]|last|<n>|<uuid>]\n\n\
     Without arguments, opens an interactive overlay to pick the message to\n\
     rewind the conversation to (↑↓ to navigate, Enter to select, y/n to confirm).\n\n\
     With arguments, rolls back file changes recorded by the shadow-git\n\
     snapshot system (absorbing the former /undo and /revert):\n\
       /rewind list     — list assistant turns with recorded file changes\n\
       /rewind diff [n] — preview a turn's diff without reverting\n\
       /rewind last     — revert the most recent assistant turn\n\
       /rewind <n>      — revert the n-th most recent assistant turn\n\
       /rewind <uuid>   — revert the turn whose message id starts with <uuid>\n\n\
     The legacy /undo and /revert commands remain hidden one-release\n\
     compatibility aliases for the argument forms."
}

async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        if ctx.messages.is_empty() {
            return CommandResult::Message(
                "Nothing to rewind — conversation is empty.".to_string(),
            );
        }
        return CommandResult::OpenRewindOverlay;
    }
    // File-rollback forms absorbed from /undo and /revert. RevertCommand
    // already handles list / diff / <n> / <uuid>.
    match trimmed {
        "last" | "undo" => RevertCommand.execute("", ctx).await,
        _ => RevertCommand.execute(trimmed, ctx).await,
    }
}
```

Also update `description()` to: `"Rewind the conversation or roll back a turn's file changes"`.

- [ ] **Step 4: Update the TUI autocomplete entry**

In `src-rust/crates/tui/src/app.rs` PROMPT_SLASH_COMMANDS, change:

```rust
("rewind", "Rewind to an earlier turn"),
```
to
```rust
(
    "rewind",
    "Rewind the conversation or roll back file changes",
),
```

- [ ] **Step 5: Update `docs/commands.md` `### /rewind` section**

Replace the section body with:

```markdown
### /rewind

Single entry point for going back in time. Without arguments, opens an
interactive overlay to pick the message to rewind the conversation to. With
arguments, rolls back file changes recorded by the shadow-git snapshot system
(absorbing the former `/undo` and `/revert`).

```
/rewind            — interactive conversation rewind overlay
/rewind list       — list assistant turns with recorded file changes
/rewind diff [n]   — preview a turn's diff without reverting
/rewind last       — revert the most recent assistant turn
/rewind <n>        — revert the n-th most recent assistant turn
/rewind <uuid>     — revert the turn whose message id starts with <uuid>
```

`/undo` and `/revert` remain hidden compatibility aliases for one release.
```

- [ ] **Step 6: Run the crate tests**

Run: `cd src-rust && cargo test -p claurst-commands`
Expected: PASS (including `prompt_slash_commands_covers_registry` and the two new tests)

- [ ] **Step 7: Commit (signed)**

```bash
git add src-rust/crates/commands/src/lib.rs src-rust/crates/tui/src/app.rs docs/commands.md docs/superpowers/plans/2026-06-10-issue-59-remainder.md
git commit -S -m "$(cat <<'EOF'
feat(commands): unify file rollback under /rewind

/rewind with arguments now forwards to the shadow-git revert paths
(list, diff, last, <n>, <uuid>) so file rollback stays discoverable
through the one visible rewind command. Bare /rewind keeps the
interactive conversation-rewind overlay. Completes the rewind bullet
of #59 that #68 only hid.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Remove the dead `ide` command end-to-end

**Files:**
- Modify: `src-rust/crates/commands/src/named_commands.rs` (delete `is_pid_alive` helper ~649–670, `IdeCommand` ~672–754, registry entry `Box::new(IdeCommand)` ~1001, test assertions referencing `ide`)
- Modify: `src-rust/crates/commands/src/lib.rs` (delete `NamedCommandAdapter` ide block ~9268, `"ide"` in `help_command_category` ~491, `ide_slash_adapter_uses_named_command` test ~9741, `"ide"` in `test_core_commands_present` expected list ~9803)
- Modify: `src-rust/crates/tui/src/app.rs` (delete `("ide", …)` PROMPT_SLASH_COMMANDS entry ~96; grep for `"ide"` in its `help_command_category` and remove if present)
- Delete: `src-rust/crates/core/src/ide.rs`
- Modify: `src-rust/crates/core/src/lib.rs` (remove `mod ide;` and the `pub use ide::{detect_ide, IdeKind};` re-export at ~line 73)
- Modify: `docs/commands.md` (remove `/ide` adapter row ~1186, `/ide` in adapter list prose ~1192, `ide` named-command table row ~1202)

- [ ] **Step 1: Delete the slash adapter, named command, helper, and core module**

In `named_commands.rs`: remove the `is_pid_alive` helper block, the whole `// ide` section (struct + impl), and `Box::new(IdeCommand),` from `NAMED_COMMANDS`.
In `lib.rs`: remove the `Box::new(NamedCommandAdapter { slash_name: "ide", … }),` block; change `"mcp" | "hooks" | "ide" | "chrome" => "Integrations",` to `"mcp" | "hooks" | "chrome" => "Integrations",`.
In `tui/src/app.rs`: remove `("ide", "Connect to the active IDE integration"),`; run `grep -n '"ide"' src-rust/crates/tui/src/app.rs` and remove any category-match occurrence.
Delete `src-rust/crates/core/src/ide.rs`; in `core/src/lib.rs` remove `mod ide;` and `pub use ide::{detect_ide, IdeKind};`.

- [ ] **Step 2: Update tests that reference `ide`**

In `lib.rs` tests: delete `ide_slash_adapter_uses_named_command` entirely; remove `"ide",` from the `expected` array in `test_core_commands_present`.
In `named_commands.rs` tests: in `test_find_named_command_found` remove `assert!(find_named_command("ide").is_some());`; in `test_find_named_command_case_insensitive` replace `assert!(find_named_command("IDE").is_some());` with `assert!(find_named_command("Agents").is_some());` already present — so change the second assertion to another existing command, e.g. `assert!(find_named_command("BRANCH").is_some());`.

- [ ] **Step 3: Verify nothing else references the removed items**

Run: `cd src-rust && grep -rn "detect_ide\|IdeKind\|IdeCommand\|extension_install_command" crates`
Expected: no matches.

- [ ] **Step 4: Update `docs/commands.md`**

Remove the `| /ide | Manage IDE integrations and show status. |` row, drop `` `/ide` `` from the adapters prose list, and remove the `| ide | Manage IDE integrations. |` named-command row.

- [ ] **Step 5: Build and test the affected crates**

Run: `cd src-rust && cargo test -p claurst-core -p claurst-commands && cargo check -p claurst`
Expected: PASS / clean check (registry tests, prompt-list test, named-command tests all green)

- [ ] **Step 6: Commit (signed)**

```bash
git add -A src-rust/crates docs/commands.md
git commit -S -m "$(cat <<'EOF'
feat(commands): remove dead ide command

Issue #59 asked to verify whether the ide integration actually works
for coven-code. It cannot: the command suggests installing a
coven-code.coven-code editor extension that is not published anywhere
(VS Code marketplace, Open VSX, OpenCoven org), and the
~/.coven-code/ide/*.lock files it reads have no writer without that
extension. Real IDE support remains the ACP surface (coven-code acp).
Removes the named command, its slash adapter, and the now-orphaned
core::ide detection module.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Full verification + PR

- [ ] **Step 1: Workspace checks**

Run: `cd src-rust && cargo fmt --all --check && cargo test -p claurst-commands -p claurst-tui -p claurst-core`
Expected: fmt clean, all tests PASS.

- [ ] **Step 2: Pre-push signature check**

```bash
git log origin/main..HEAD --pretty='%H %G?' | awk '$2 != "G" {print "UNSIGNED:", $0}'
```
Expected: no output.

- [ ] **Step 3: Push branch and open PR**

```bash
git push -u origin issue-59-phase2-command-cut
gh pr create --title "feat(commands): finish #59 remainder — unify /rewind, remove dead ide" --body "…(summarize Tasks 1–2, link #59 and #68, include ide verification evidence)…"
```

- [ ] **Step 4: Comment on issue #59** with the `ide` verification evidence and a pointer to the follow-up PR (the issue was closed by #68 while two bullets were incomplete).
