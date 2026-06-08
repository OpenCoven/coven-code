export const meta = { title: 'Slash Commands' };

export function render() {
  return `
    <h1>Slash commands</h1>
    <p class="lead">Coven Code ships with 70+ slash commands organised into categories — session control, model selection, configuration, code &amp; git workflows, agents, MCP, and more. Type <code>/</code> in the chat input to open the palette, or <kbd>Ctrl+K</kbd> from anywhere.</p>

    <h2>Resolution order</h2>

    <p>When you type a command, the registry checks in priority order:</p>

    <pre><code data-lang="bash">built-in commands → user command templates → discovered skills → plugin commands</code></pre>

    <p>The first match wins, so user templates can override built-ins of the same name.</p>

    <h2>Session &amp; navigation</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/help</code></td><td>Show all commands</td></tr>
        <tr><td><code>/clear</code></td><td>Clear the conversation</td></tr>
        <tr><td><code>/exit</code></td><td>Quit the session</td></tr>
        <tr><td><code>/resume</code></td><td>Resume a previous session</td></tr>
        <tr><td><code>/session</code></td><td>List or pick a session</td></tr>
        <tr><td><code>/fork</code></td><td>Branch the current session into a new one</td></tr>
        <tr><td><code>/rename</code></td><td>Rename the current session</td></tr>
        <tr><td><code>/rewind</code></td><td>Go back to a previous message</td></tr>
        <tr><td><code>/compact</code></td><td>Compress conversation history</td></tr>
      </tbody>
    </table>

    <h2>Model &amp; provider</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/model</code></td><td>Switch model or provider</td></tr>
        <tr><td><code>/providers</code></td><td>List available providers</td></tr>
        <tr><td><code>/connect</code></td><td>Connect to a remote provider endpoint</td></tr>
        <tr><td><code>/thinking</code></td><td>Toggle extended thinking display</td></tr>
        <tr><td><code>/effort</code></td><td>Set extended-thinking effort level</td></tr>
        <tr><td><code>/advisor &lt;model&gt;</code></td><td>Set a secondary advisor model</td></tr>
        <tr><td><code>/fast</code></td><td>Toggle fast mode (smaller, faster model)</td></tr>
      </tbody>
    </table>

    <h2>Configuration</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/config</code></td><td>Open the settings editor</td></tr>
        <tr><td><code>/keybindings</code></td><td>Open the interactive keybinding editor</td></tr>
        <tr><td><code>/permissions</code></td><td>Manage tool permission rules</td></tr>
        <tr><td><code>/hooks</code></td><td>Inspect active hooks</td></tr>
        <tr><td><code>/mcp</code></td><td>Manage MCP servers</td></tr>
        <tr><td><code>/output-style</code> · <code>/theme</code> · <code>/statusline</code></td><td>Visual customisation</td></tr>
        <tr><td><code>/vim</code></td><td>Toggle vim mode</td></tr>
        <tr><td><code>/voice</code></td><td>Voice input mode</td></tr>
      </tbody>
    </table>

    <h2>Code &amp; git</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/commit</code></td><td>Stage and commit current changes</td></tr>
        <tr><td><code>/diff</code></td><td>Show working-tree diff</td></tr>
        <tr><td><code>/undo</code></td><td>Undo the last file edit (uses snapshot)</td></tr>
        <tr><td><code>/review</code></td><td>Review a PR or current changes</td></tr>
        <tr><td><code>/security-review</code></td><td>Security audit of pending changes</td></tr>
        <tr><td><code>/init</code></td><td>Initialise a new <code>AGENTS.md</code> for the project</td></tr>
        <tr><td><code>/search</code></td><td>Codebase search</td></tr>
      </tbody>
    </table>

    <h2>Memory, context, cost</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/memory</code></td><td>Browse persistent memory entries</td></tr>
        <tr><td><code>/context</code></td><td>Show context window usage</td></tr>
        <tr><td><code>/cost</code></td><td>Token usage and dollar cost for the session</td></tr>
        <tr><td><code>/usage</code> · <code>/stats</code> · <code>/insights</code></td><td>Session statistics</td></tr>
        <tr><td><code>/status</code></td><td>Connection &amp; daemon status</td></tr>
      </tbody>
    </table>

    <h2>Agents &amp; tasks</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/agents</code></td><td>List built-in, custom, and familiar agents</td></tr>
        <tr><td><code>/agent &lt;name&gt;</code></td><td>Switch active agent for this session</td></tr>
        <tr><td><code>/tasks</code></td><td>Show the live task list</td></tr>
        <tr><td><code>/goal &lt;objective&gt;</code></td><td>Set an autonomous multi-turn goal</td></tr>
        <tr><td><code>/managed-agents</code></td><td>Configure manager-executor agents</td></tr>
        <tr><td><code>/plan</code> · <code>/ultraplan</code></td><td>Enter planning mode</td></tr>
        <tr><td><code>/ultrareview</code></td><td>Exhaustive multi-dimensional code review</td></tr>
      </tbody>
    </table>

    <h2>Auth</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/login</code></td><td>OAuth login (<code>--codex</code> for ChatGPT, <code>--label</code> to name)</td></tr>
        <tr><td><code>/accounts</code></td><td>List stored profiles</td></tr>
        <tr><td><code>/switch &lt;id&gt;</code></td><td>Switch active account</td></tr>
        <tr><td><code>/logout</code></td><td>Clear credentials</td></tr>
        <tr><td><code>/refresh</code></td><td>Refresh OAuth tokens</td></tr>
      </tbody>
    </table>

    <h2>Display</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/caveman</code></td><td>Telegraphic speech mode (saves 40–85% tokens)</td></tr>
        <tr><td><code>/rocky</code></td><td>Rocky (Project Hail Mary) speech mode</td></tr>
        <tr><td><code>/normal</code></td><td>Deactivate speech modes</td></tr>
        <tr><td><code>/mobile</code></td><td>Compact mobile-friendly rendering</td></tr>
        <tr><td><code>/color</code></td><td>Adjust colour palette at runtime</td></tr>
      </tbody>
    </table>

    <h2>Coven substrate</h2>

    <p>When the Coven daemon is online, these are wired up:</p>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/coven</code></td><td>Substrate surface: <code>kill</code>, <code>log</code>, <code>send</code>, <code>familiars</code>, etc.</td></tr>
        <tr><td><code>/familiar</code></td><td>Switch active familiar (also <kbd>F2</kbd>)</td></tr>
        <tr><td><code>/handoff</code></td><td>Hand off a session between familiars</td></tr>
      </tbody>
    </table>

    <h2>Diagnostics</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/doctor</code></td><td>Environment and substrate health check</td></tr>
        <tr><td><code>/version</code></td><td>Show version info</td></tr>
        <tr><td><code>/update</code></td><td>Check for and download updates</td></tr>
        <tr><td><code>/export</code></td><td>Save session transcript</td></tr>
        <tr><td><code>/copy</code></td><td>Copy last response to clipboard</td></tr>
      </tbody>
    </table>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/commands.md" target="_blank" rel="noopener">the full slash commands reference</a> for every flag, behaviour detail, and the planning/internal commands (<code>/summary</code>, <code>/brief</code>, <code>/sandbox-toggle</code>, <code>/think-back</code>, <code>/thinkback-play</code>, etc.).</p>
  `;
}
