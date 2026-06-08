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

    <div class="demo">
      <div class="demo-header">
        <span>effort scale · low → max</span>
      </div>
      <div class="demo-body">
        <div class="effort-scale">
          <div class="effort-step">
            <div class="effort-step-bar">
              <span class="effort-step-tick on"></span>
              <span class="effort-step-tick"></span>
              <span class="effort-step-tick"></span>
              <span class="effort-step-tick"></span>
            </div>
            <div class="effort-step-name">low</div>
            <div class="effort-step-desc">Minimal thinking; fastest responses.</div>
            <div class="effort-step-meta">persisted</div>
          </div>
          <div class="effort-step">
            <div class="effort-step-bar">
              <span class="effort-step-tick on"></span>
              <span class="effort-step-tick on"></span>
              <span class="effort-step-tick"></span>
              <span class="effort-step-tick"></span>
            </div>
            <div class="effort-step-name">medium</div>
            <div class="effort-step-desc">Moderate reasoning; balanced speed + quality.</div>
            <div class="effort-step-meta">persisted</div>
          </div>
          <div class="effort-step">
            <div class="effort-step-bar">
              <span class="effort-step-tick on"></span>
              <span class="effort-step-tick on"></span>
              <span class="effort-step-tick on"></span>
              <span class="effort-step-tick"></span>
            </div>
            <div class="effort-step-name">high</div>
            <div class="effort-step-desc">Deep reasoning; best quality for most tasks. API default.</div>
            <div class="effort-step-meta">persisted</div>
          </div>
          <div class="effort-step">
            <div class="effort-step-bar">
              <span class="effort-step-tick on"></span>
              <span class="effort-step-tick on"></span>
              <span class="effort-step-tick on"></span>
              <span class="effort-step-tick on"></span>
            </div>
            <div class="effort-step-name">max</div>
            <div class="effort-step-desc">Maximum budget — Opus-class only. Falls back to high on unsupported models.</div>
            <div class="effort-step-meta">session only</div>
          </div>
        </div>
      </div>
    </div>

    <p>Override with the <code>CLAUDE_CODE_EFFORT_LEVEL</code> env var; conflicts surface a warning when you run <code>/effort</code>.</p>

    <h2>Auto-Compaction</h2>

    <p>The context window is finite. Auto-compaction summarises history when usage approaches the limit (effective window minus a 13,000-token buffer), keeping the session alive without manual intervention.</p>

    <div class="demo">
      <div class="demo-header">
        <span>auto-compaction flow</span>
      </div>
      <div class="demo-body">
        <div class="lifecycle">
          <div class="lifecycle-phase">
            <div class="lifecycle-phase-head">
              <span class="lifecycle-phase-name">Triggered</span>
              <span class="lifecycle-phase-when">usage crosses the auto-compact threshold</span>
            </div>
            <div class="lifecycle-track">
              <span class="lifecycle-event"><span class="lifecycle-event-dot"></span>turn N completes</span>
              <span class="lifecycle-arrow">tokens checked</span>
              <span class="lifecycle-event"><span class="lifecycle-event-dot"></span>PreCompact hook</span>
              <span class="lifecycle-arrow">summarise</span>
              <span class="lifecycle-event"><span class="lifecycle-event-dot"></span>PostCompact hook</span>
              <span class="lifecycle-arrow">resume</span>
              <span class="lifecycle-event"><span class="lifecycle-event-dot"></span>turn N+1</span>
            </div>
            <p class="lifecycle-note">PreCompact exit code 2 blocks the compaction; the session continues until you free space manually with <code>/compact</code>.</p>
          </div>
        </div>
      </div>
    </div>

    <pre><code data-lang="bash">DISABLE_AUTO_COMPACT=1 coven-code               # disable auto, keep /compact
DISABLE_COMPACT=1 coven-code                    # disable entirely
CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=80 coven-code   # custom threshold</code></pre>

    <p>Manual compaction with optional guidance:</p>

    <pre><code data-lang="bash">/compact focus on the database schema changes</code></pre>

    <h2>Context Window</h2>

    <pre><code data-lang="bash">/context     # show usage relative to the model's window</code></pre>

    <div class="demo">
      <div class="demo-header">
        <span>context window meter · 200k example</span>
      </div>
      <div class="demo-body">
        <div class="ctx-meter">
          <div class="ctx-bar">
            <span class="ctx-bar-tick" style="left: 0%;">0</span>
            <span class="ctx-bar-tick" style="left: 90%;">180k</span>
            <span class="ctx-bar-tick" style="left: 98.5%;">197k</span>
            <span class="ctx-bar-tick" style="left: 100%;">200k</span>
            <span class="ctx-bar-zone ctx-bar-safe">Safe</span>
            <span class="ctx-bar-zone ctx-bar-warn">Warn / Error</span>
            <span class="ctx-bar-zone ctx-bar-block">Block</span>
          </div>
          <div class="ctx-legend">
            <div class="ctx-legend-item ctx-legend-item-safe">
              <div class="ctx-legend-name">Safe · 0 → limit−20k</div>
              <div class="ctx-legend-desc">Plenty of headroom. Auto-compact won't fire yet.</div>
            </div>
            <div class="ctx-legend-item ctx-legend-item-warn">
              <div class="ctx-legend-name">Warning / Error · last 20k</div>
              <div class="ctx-legend-desc">Status line escalates from warning to error visual. Auto-compact triggers around here.</div>
            </div>
            <div class="ctx-legend-item ctx-legend-item-block">
              <div class="ctx-legend-name">Block · last 3k</div>
              <div class="ctx-legend-desc">Further input blocked until you compact, either manually with <code>/compact</code> or by waiting for auto-compact.</div>
            </div>
          </div>
        </div>
      </div>
    </div>

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
