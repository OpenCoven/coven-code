# Query agent and scheduler lifetime disposition

Audit base: `d875fdc` on 2026-10-18. Owner: Val, with implementation in
`src-rust/crates/query`, `src-rust/crates/tools`, and the CLI entrypoint.

**Disposition: retain background agents and scheduled queries, with lifecycle
repair required before accepting this coverage row.** They provide delegated
work and scheduled prompts, but current main does not prove that all child work
is owned through cancellation and shutdown. The boundaries below need separate
reproductions and reviewed fixes; this record does not close them.

## Evidence on current main

- [`agent_tool.rs`](../src-rust/crates/query/src/agent_tool.rs) registers a
  background agent with a cancellation token. Its isolated test confirms that
  Monitor cancellation closes a pending provider request. The synchronous
  path instead awaits the child query inside `execute`; parent cancellation
  can drop that future before the following worktree cleanup. This cleanup
  path has not been reproduced under cancellation.
- Agent worktree isolation creates a detached worktree, then calls
  `git worktree remove --force` after either synchronous or background child
  completion. In a disposable repository, a detached worktree with an
  uncommitted file was removed by `git worktree remove --force <path>`:
  `dirty_before=True`, `worktree_after=False`, and
  `dirty_file_after=False`. The current result contains no preservation or
  handoff record for those edits or a detached commit.
- [`cron_scheduler.rs`](../src-rust/crates/query/src/cron_scheduler.rs)
  detaches the scheduler task and each due query. The CLI
  [`main.rs`](../src-rust/crates/cli/src/main.rs) cancels their shared token on
  exit but does not await these tasks. The scheduler has no active-query cap;
  the 50-job limit in [`cron.rs`](../src-rust/crates/tools/src/cron.rs) counts
  scheduled definitions, not running queries. Repeated ticks can therefore
  launch overlapping runs while a previous run remains pending.
- `pop_due_tasks` removes one-shot tasks before execution and documents that
  a crash between persistence and delivery can lose a run. Its write failure
  path withholds durable one-shots. This is an explicit at-most-once behavior
  that needs a product decision before changing retry semantics.
- AutoDream completion has a separate open fix in #366 at this audit base;
  its PR and hosted checks must be assessed independently.

## Required gates

1. Reproduce isolated agent completion with dirty edits and a detached commit,
   then reproduce parent cancellation during a synchronous child. Preserve
   any resulting worktree until its content is handed off or backed up; report
   its path and cleanup outcome to the caller. Val owns the retention policy
   and the reviewed implementation.
2. Reproduce two scheduled ticks with a blocked first query and shutdown with
   a live child. Define an active-query limit, overload response, and bounded
   shutdown join without dropping one-shot tasks silently. Val owns the
   scheduling contract and the reviewed implementation.
3. Recheck Monitor cancellation, foreground agent completion, one-shot
   persistence, and #366 on the resulting main tree. Keep human acceptance
   for agent worktree handoff separate from automated fixture proof.
