export const meta = { title: 'Hooks' };

export function render() {
  return `
    <h1>Hooks</h1>
    <p class="lead">Hooks are executable logic Coven Code calls at lifecycle events — before a tool runs, after a turn completes, when the session starts. Hooks can be shell commands, LLM evaluations, sub-agent verifications, or HTTP POSTs.</p>

    <h2>How hooks work</h2>

    <p>When an event fires, Coven Code:</p>

    <ol>
      <li>Serialises a JSON payload describing the event</li>
      <li>Passes that JSON to the hook's stdin (or HTTP body)</li>
      <li>Waits for the hook to exit, unless marked <code>async</code></li>
      <li>Interprets the exit code or response according to the event's blocking rules</li>
    </ol>

    <p>Because every hook receives structured JSON and returns a plain exit code, hooks can be written in any language that reads stdin and writes stderr/stdout.</p>

    <h2>Hook types</h2>

    <h3><code>command</code> — shell command</h3>

    <pre><code data-lang="json">{
  "type": "command",
  "command": "bash /path/to/my-hook.sh"
}</code></pre>

    <p>Runs the string through the configured shell (<code>bash</code> by default, or <code>powershell</code>).</p>

    <h3><code>prompt</code> — LLM evaluation</h3>

    <pre><code data-lang="json">{
  "type": "prompt",
  "prompt": "Does this tool call look safe? $ARGUMENTS"
}</code></pre>

    <p>Sends the event payload to a lightweight model. Must respond <code>{"ok": true}</code> to pass, <code>{"ok": false, "reason": "..."}</code> to fail. Defaults to the fastest available small model.</p>

    <h3><code>agent</code> — agentic verifier</h3>

    <pre><code data-lang="json">{
  "type": "agent",
  "prompt": "Verify that the unit tests passed. Use $ARGUMENTS for context."
}</code></pre>

    <p>Spawns a short-lived agent session for verification. Like <code>prompt</code>, expects a <code>SyntheticOutput</code> tool call with <code>{"ok", "reason"}</code>.</p>

    <h3><code>http</code> — HTTP POST</h3>

    <pre><code data-lang="json">{
  "type": "http",
  "url": "https://hooks.example.com/coven-code",
  "headers": {
    "Authorization": "Bearer $SLACK_TOKEN"
  },
  "allowedEnvVars": ["SLACK_TOKEN"]
}</code></pre>

    <p>POSTs the event payload JSON to a URL. Header values may reference env vars using <code>$VAR</code> or <code>\${VAR}</code>, but only env vars listed in <code>allowedEnvVars</code> are interpolated.</p>

    <h2>Common fields</h2>

    <table>
      <thead><tr><th>Field</th><th>Purpose</th></tr></thead>
      <tbody>
        <tr><td><code>timeout</code></td><td>Per-hook timeout in seconds</td></tr>
        <tr><td><code>statusMessage</code></td><td>Custom spinner text shown while the hook runs</td></tr>
        <tr><td><code>async</code></td><td>Run in the background without blocking the event</td></tr>
        <tr><td><code>asyncRewake</code></td><td>Background hook that wakes the model on exit code 2</td></tr>
        <tr><td><code>once</code></td><td>Remove from the session after first fire</td></tr>
        <tr><td><code>if</code></td><td>Permission-rule-style filter (e.g. <code>"Bash(git *)"</code>)</td></tr>
      </tbody>
    </table>

    <h2>Hook events</h2>

    <p>The full event set, organised by phase:</p>

    <table>
      <thead><tr><th>Phase</th><th>Events</th></tr></thead>
      <tbody>
        <tr><td>Tool lifecycle</td><td><code>PreToolUse</code>, <code>PostToolUse</code>, <code>PostToolUseFailure</code></td></tr>
        <tr><td>Turn lifecycle</td><td><code>UserPromptSubmit</code>, <code>Stop</code>, <code>StopFailure</code>, <code>Notification</code></td></tr>
        <tr><td>Session lifecycle</td><td><code>SessionStart</code>, <code>SessionEnd</code></td></tr>
        <tr><td>Subagent lifecycle</td><td><code>SubagentStart</code>, <code>SubagentStop</code></td></tr>
        <tr><td>Compaction</td><td><code>PreCompact</code>, <code>PostCompact</code></td></tr>
        <tr><td>Permissions</td><td><code>PermissionRequest</code>, <code>PermissionDenied</code></td></tr>
        <tr><td>Tasks</td><td><code>TaskCreated</code>, <code>TaskCompleted</code></td></tr>
        <tr><td>Elicitation</td><td><code>Elicitation</code>, <code>ElicitationResult</code></td></tr>
        <tr><td>Other</td><td><code>ConfigChange</code>, <code>WorktreeCreate</code></td></tr>
      </tbody>
    </table>

    <p>Most events use exit code <code>0</code> for success, <code>1</code> for failure (block + report), and <code>2</code> for "rewake the model with this stderr as feedback."</p>

    <h2>Where hooks live</h2>

    <ul>
      <li><strong>User-level</strong>: <code>~/.coven-code/hooks.json</code></li>
      <li><strong>Project-level</strong>: <code>.coven-code/hooks.json</code> in the project root</li>
      <li><strong>Plugin-provided</strong>: inline in <code>plugin.toml</code>/<code>plugin.json</code> or in a plugin's <code>hooks/</code> directory — see <a href="#plugins">Plugins</a></li>
    </ul>

    <p>Inspect active hooks with <code>/hooks</code> inside the TUI.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/hooks.md" target="_blank" rel="noopener">the full hooks reference</a> for per-event payload schemas, matcher syntax, blocking rules, and complete examples.</p>
  `;
}
