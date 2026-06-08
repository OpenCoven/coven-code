export const meta = { title: 'Advanced' };

export function render() {
  return `
    <h1>Advanced features</h1>
    <p class="lead">Extended thinking, auto-compaction, context management, session continuity, plan mode, worktrees, goals, and managed agents. The escape hatches for when you've outgrown the defaults.</p>

    <h2>Extended thinking</h2>

    <p>Gives the model additional computation budget to reason through hard problems before responding.</p>

    <pre><code data-lang="bash">/thinking              # toggle on/off
/effort &lt;level&gt;        # low / medium / high / max

coven-code --thinking &lt;tokens&gt;   # specific token budget
coven-code --effort &lt;level&gt;</code></pre>

    <table>
      <thead><tr><th>Level</th><th>Use</th></tr></thead>
      <tbody>
        <tr><td><code>low</code></td><td>Minimal thinking; fastest responses</td></tr>
        <tr><td><code>medium</code></td><td>Moderate reasoning; balanced</td></tr>
        <tr><td><code>high</code></td><td>Deep reasoning; best quality for most tasks</td></tr>
        <tr><td><code>max</code></td><td>Maximum budget; Opus-class models only (falls back to <code>high</code>)</td></tr>
      </tbody>
    </table>

    <p><code>low</code>, <code>medium</code>, <code>high</code> persist across sessions; <code>max</code> is session-scoped. Override with <code>CLAUDE_CODE_EFFORT_LEVEL</code>.</p>

    <h2>Auto-compaction</h2>

    <p>The context window is finite. Auto-compaction summarises history when usage approaches the limit (effective window minus a 13,000-token buffer), keeping the session alive without manual intervention.</p>

    <p>Hooks <code>PreCompact</code> and <code>PostCompact</code> fire around the operation; <code>PreCompact</code> can block via exit code 2.</p>

    <pre><code data-lang="bash">DISABLE_AUTO_COMPACT=1 coven-code               # disable auto, keep /compact
DISABLE_COMPACT=1 coven-code                    # disable entirely
CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=80 coven-code   # custom threshold</code></pre>

    <p>Manual compaction with optional guidance:</p>

    <pre><code data-lang="bash">/compact focus on the database schema changes</code></pre>

    <h2>Context window</h2>

    <pre><code data-lang="bash">/context     # show usage relative to the model's window</code></pre>

    <ul>
      <li><strong>Warning threshold:</strong> 20,000 tokens before the limit</li>
      <li><strong>Error threshold:</strong> 20,000 tokens before — more prominent visual</li>
      <li><strong>Blocking limit:</strong> 3,000 tokens before — further input blocked until compaction</li>
    </ul>

    <p>External viewer: <code>ctx-viz</code> for inspecting transcripts outside the TUI.</p>

    <h2>Session management</h2>

    <p>Every session is persisted as a JSONL transcript. List, resume, fork, rename:</p>

    <pre><code data-lang="bash">/session     # list / pick
/resume      # resume the last
/fork        # branch the current
/rename      # rename
/rewind      # step back to a previous turn</code></pre>

    <p>Transcripts live under <code>~/.coven-code/sessions/&lt;project-hash&gt;/&lt;session-id&gt;.jsonl</code>. The SDK exposes session APIs for tooling.</p>

    <h2>Worktrees</h2>

    <p>Coven Code's worktree tools (<code>WorktreeCreate</code>, <code>WorktreeRemove</code>, <code>WorktreeList</code>) integrate with git worktrees so a session can spin off an isolated branch + checkout without changing your working tree. Custom worktree backends can be registered via plugins.</p>

    <h2>Plan mode</h2>

    <p>Read-only mode — the model can read, search, and reason but cannot write or run shell. Useful for review and design discussions before committing to implementation.</p>

    <pre><code data-lang="bash">/plan
coven-code --permission-mode plan "design the auth refactor"</code></pre>

    <h2>Goal system</h2>

    <p>Set a durable objective that survives across turns. Coven Code works autonomously until the goal is verified complete via the <code>GoalCompleteTool</code> — audited completion rather than just stopping when the model thinks it's done.</p>

    <pre><code data-lang="bash">/goal Migrate all snake_case API responses to camelCase</code></pre>

    <table>
      <thead><tr><th>Status</th><th>Meaning</th></tr></thead>
      <tbody>
        <tr><td><code>active</code></td><td>Currently being worked on</td></tr>
        <tr><td><code>blocked</code></td><td>Waiting on user input or an external dependency</td></tr>
        <tr><td><code>verified</code></td><td>Completed and verified by <code>GoalCompleteTool</code></td></tr>
        <tr><td><code>cancelled</code></td><td>Explicitly cancelled by the user</td></tr>
      </tbody>
    </table>

    <p>Disable the system entirely with <code>config.goal_system_enabled: false</code>.</p>

    <h2>Managed agents (preview)</h2>

    <p>Manager-executor architecture: a manager model delegates subtasks to parallel executor agents with budget split controls.</p>

    <pre><code data-lang="bash">/managed-agents</code></pre>

    <p>The TUI walks you through enabling, choosing a manager model, and configuring executor pools.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/advanced.md" target="_blank" rel="noopener">the full advanced reference</a> for JSONL transcript schema, SDK examples, custom worktree backends, and the complete managed-agents configuration.</p>
  `;
}
