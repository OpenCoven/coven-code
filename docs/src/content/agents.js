export const meta = { title: 'Agents' };

export function render() {
  return `
    <h1>Agents &amp; Multi-Agent</h1>
    <p class="lead">Coven Code ships with three built-in named agents and supports user-defined agents, coordinator-mode parallelism, and a managed-agents preview where a manager model delegates to executor agents.</p>

    <h2>Built-In Agents</h2>

    <table>
      <thead><tr><th>Agent</th><th>Access</th><th>Max turns</th><th>Use</th></tr></thead>
      <tbody>
        <tr><td><code>build</code></td><td>full — all tools</td><td>unlimited</td><td>Implement features, fix bugs</td></tr>
        <tr><td><code>plan</code></td><td>read-only — no writes or shell</td><td>20</td><td>Analyze codebases, plan changes</td></tr>
        <tr><td><code>explore</code></td><td>search-only — search tools only</td><td>15</td><td>Quickly locate code, answer structure questions</td></tr>
      </tbody>
    </table>

    <h2>Selecting an Agent</h2>

    <pre><code data-lang="bash">coven-code --agent build "implement the OAuth2 login flow"
coven-code --agent plan "analyze the database schema and suggest improvements"
coven-code --agent explore "find all usages of the deprecated config API"</code></pre>

    <p>Combine with <code>--provider</code> and <code>--model</code>:</p>

    <pre><code data-lang="bash">coven-code --agent plan --provider openai --model o3 "review this architecture"</code></pre>

    <p>Inside the TUI, use <code>/agents</code> to list everything available (built-in, custom, plus Coven familiars when the daemon is online).</p>

    <h2>Custom Agents</h2>

    <p>Define custom agents in <code>~/.coven-code/settings.json</code> under <code>agents</code>. Custom definitions override built-ins with the same name.</p>

    <pre><code data-lang="json">{
  "agents": {
    "review": {
      "description": "Senior code reviewer focused on correctness and security",
      "model": "anthropic/claude-opus-4-6",
      "temperature": 0.3,
      "access": "read-only",
      "max_turns": 30,
      "color": "magenta",
      "system_prompt": "You are a senior code reviewer. Focus on correctness, security, and maintainability."
    }
  }
}</code></pre>

    <p>Workspace agents can also be defined as markdown files in <code>.coven-code/agents/*.md</code> with frontmatter — these are picked up automatically and surfaced in <code>/agents</code>.</p>

    <h2>Coordinator Mode</h2>

    <p>Coordinator mode runs a manager model that dispatches tasks to worker agents in parallel. Enable it with:</p>

    <pre><code data-lang="bash">coven-code --coordinator "implement the entire user-management module"</code></pre>

    <p>The coordinator has access to coordinator-only tools (Spawn, Wait, TaskCreate / TaskUpdate / TaskList) and cannot directly edit files or run shell — that work is done by spawned workers. Workers run with the standard tool set minus the coordinator tools.</p>

    <h2>Coven Familiars as Agents</h2>

    <p>When the Coven daemon is running, every familiar in <code>~/.coven/familiars.toml</code> is automatically surfaced as a selectable agent. See <a href="#familiars">Familiars</a> for details.</p>

    <h2>Managed Agents (Preview)</h2>

    <p>Configure a manager-executor architecture with <code>/managed-agents</code>. The manager delegates subtasks to parallel executor agents with full budget split controls.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/agents.md" target="_blank" rel="noopener">the full agents reference</a> for the complete <code>AgentDefinition</code> schema, coordinator-only tools, worker tool sets, banned tools in coordinator mode, and managed-agents configuration.</p>
  `;
}
