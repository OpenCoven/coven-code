export const meta = { title: 'Advanced' };

export function render() {
  return `
    <h1>Advanced Features</h1>
    <p class="lead">Extended thinking, auto-compaction, context management, session continuity, plan mode, worktrees, goals, and managed agents. The escape hatches for when you've outgrown the defaults.</p>

    <h2>Extended Thinking</h2>

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

    <h2>Auto-Compaction</h2>

    <p>The context window is finite. Auto-compaction summarises history when usage approaches the limit (effective window minus a 13,000-token buffer), keeping the session alive without manual intervention.</p>

    <p>Hooks <code>PreCompact</code> and <code>PostCompact</code> fire around the operation; <code>PreCompact</code> can block via exit code 2.</p>

    <pre><code data-lang="bash">DISABLE_AUTO_COMPACT=1 coven-code               # disable auto, keep /compact
DISABLE_COMPACT=1 coven-code                    # disable entirely
CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=80 coven-code   # custom threshold</code></pre>

    <p>Manual compaction with optional guidance:</p>

    <pre><code data-lang="bash">/compact focus on the database schema changes</code></pre>

    <h2>Context Window</h2>

    <pre><code data-lang="bash">/context     # show usage relative to the model's window</code></pre>

    <ul>
      <li><strong>Warning threshold:</strong> 20,000 tokens before the limit</li>
      <li><strong>Error threshold:</strong> 20,000 tokens before — more prominent visual</li>
      <li><strong>Blocking limit:</strong> 3,000 tokens before — further input blocked until compaction</li>
    </ul>

    <p>External viewer: <code>ctx-viz</code> for inspecting transcripts outside the TUI.</p>

    <h2>Session Management</h2>

    <p>Every session is persisted as a JSONL transcript. List, resume, fork, rename:</p>

    <pre><code data-lang="bash">/session     # list / pick
/resume      # resume the last
/fork        # branch the current
/rename      # rename
/rewind      # step back to a previous turn</code></pre>

    <p>Transcripts live under <code>~/.coven-code/sessions/&lt;project-hash&gt;/&lt;session-id&gt;.jsonl</code>. The SDK exposes session APIs for tooling.</p>

    <h2>Worktrees</h2>

    <p>Coven Code's worktree tools (<code>WorktreeCreate</code>, <code>WorktreeRemove</code>, <code>WorktreeList</code>) integrate with git worktrees so a session can spin off an isolated branch + checkout without changing your working tree. Custom worktree backends can be registered via plugins.</p>

    <h2>Plan Mode</h2>

    <p>Read-only mode — the model can read, search, and reason but cannot write or run shell. Useful for review and design discussions before committing to implementation.</p>

    <pre><code data-lang="bash">/plan
coven-code --permission-mode plan "design the auth refactor"</code></pre>

    <h2>Goal System</h2>

    <p>Set a durable objective that survives across turns. Coven Code works autonomously until the goal is verified complete via the <code>GoalCompleteTool</code> — audited completion rather than just stopping when the model thinks it's done.</p>

    <pre><code data-lang="bash">/goal Migrate all snake_case API responses to camelCase</code></pre>

    <div class="demo">
      <div class="demo-header">
        <span>goal state machine</span>
      </div>
      <div class="demo-body">
        <div class="state-diagram">
          <div class="state-node state-node-active">
            <div class="state-node-name">active</div>
            <div class="state-node-desc">Currently being worked on across turns</div>
          </div>
          <div class="state-node state-node-blocked">
            <div class="state-node-name">blocked</div>
            <div class="state-node-desc">Waiting on user input or an external dependency</div>
          </div>
          <div class="state-node state-node-terminal">
            <div class="state-node-name">verified</div>
            <div class="state-node-desc">Completed and audited by <code>GoalCompleteTool</code></div>
          </div>
          <div class="state-node state-node-terminal" style="grid-column: 3;">
            <div class="state-node-name">cancelled</div>
            <div class="state-node-desc">User explicitly cancelled with <code>/goal cancel</code></div>
          </div>
        </div>
        <div class="state-transitions">
          <div class="state-transition">
            <span class="state-transition-from">(start)</span>
            <span class="state-transition-arrow">→</span>
            <span class="state-transition-to">active</span>
            <span class="state-transition-trigger">You run <code>/goal &lt;objective&gt;</code></span>
          </div>
          <div class="state-transition">
            <span class="state-transition-from">active</span>
            <span class="state-transition-arrow">→</span>
            <span class="state-transition-to">blocked</span>
            <span class="state-transition-trigger">Coven needs input or hits an external dependency</span>
          </div>
          <div class="state-transition">
            <span class="state-transition-from">blocked</span>
            <span class="state-transition-arrow">→</span>
            <span class="state-transition-to">active</span>
            <span class="state-transition-trigger">You unblock it — answer the question, fix the dependency</span>
          </div>
          <div class="state-transition">
            <span class="state-transition-from">active</span>
            <span class="state-transition-arrow">→</span>
            <span class="state-transition-to">verified</span>
            <span class="state-transition-trigger"><code>GoalCompleteTool</code> fires — audited completion, not just "the model said it's done"</span>
          </div>
          <div class="state-transition">
            <span class="state-transition-from">active / blocked</span>
            <span class="state-transition-arrow">→</span>
            <span class="state-transition-to">cancelled</span>
            <span class="state-transition-trigger">You run <code>/goal cancel</code></span>
          </div>
        </div>
      </div>
    </div>

    <p>Disable the system entirely with <code>config.goal_system_enabled: false</code>.</p>

    <h2>Managed Agents (Preview)</h2>

    <p>Manager-executor architecture: a manager model delegates subtasks to parallel executor agents with budget split controls.</p>

    <pre><code data-lang="bash">/managed-agents</code></pre>

    <p>The TUI walks you through enabling, choosing a manager model, and configuring executor pools.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/advanced.md" target="_blank" rel="noopener">the full advanced reference</a> for JSONL transcript schema, SDK examples, custom worktree backends, and the complete managed-agents configuration.</p>
  `;
}
