export const meta = { title: 'Tools' };

export function render() {
  return `
    <h1>Tools reference</h1>
    <p class="lead">Coven Code ships with 40+ built-in tools across file ops, shell execution, search, web, task management, git, notebooks, and desktop automation. Each tool is gated by a permission level and the active permission mode.</p>

    <h2>Permission levels</h2>

    <table>
      <thead><tr><th>Level</th><th>Description</th><th>Examples</th></tr></thead>
      <tbody>
        <tr><td><strong>None</strong></td><td>No external effects</td><td><code>SleepTool</code></td></tr>
        <tr><td><strong>ReadOnly</strong></td><td>Reads data; no writes or execution</td><td><code>FileReadTool</code>, <code>GlobTool</code>, <code>WebFetchTool</code></td></tr>
        <tr><td><strong>Write</strong></td><td>Creates or modifies data</td><td><code>FileWriteTool</code>, <code>FileEditTool</code>, <code>ConfigTool</code></td></tr>
        <tr><td><strong>Execute</strong></td><td>Runs code or spawns processes</td><td><code>BashTool</code>, <code>TaskCreateTool</code>, <code>SendMessageTool</code></td></tr>
        <tr><td><strong>Dangerous</strong></td><td>Broad system access; high blast radius</td><td><code>ComputerUseTool</code></td></tr>
      </tbody>
    </table>

    <h2>Permission modes</h2>

    <table>
      <thead><tr><th>Mode</th><th>Behavior</th></tr></thead>
      <tbody>
        <tr><td><code>default</code></td><td>Prompts for any tool that isn't pre-approved</td></tr>
        <tr><td><code>plan</code></td><td>All write/execute blocked; read-only runs freely</td></tr>
        <tr><td><code>auto</code></td><td>Non-destructive tools run silently; destructive prompt</td></tr>
        <tr><td><code>acceptEdits</code></td><td>File edits auto-approved; shell still prompts</td></tr>
        <tr><td><code>bypassPermissions</code></td><td>All tools run without prompting (headless/CI only)</td></tr>
      </tbody>
    </table>

    <p>Permission rules are evaluated per-project and per-user — first match wins. Manage them with <code>/permissions</code>.</p>

    <h2>File tools</h2>

    <table>
      <thead><tr><th>Tool</th><th>Level</th><th>Purpose</th></tr></thead>
      <tbody>
        <tr><td><code>FileReadTool</code></td><td>ReadOnly</td><td>Read files (text, images, PDFs, notebooks). Tracks reads for write enforcement.</td></tr>
        <tr><td><code>FileWriteTool</code></td><td>Write</td><td>Create or overwrite a file. Requires prior read unless file is new.</td></tr>
        <tr><td><code>FileEditTool</code></td><td>Write</td><td>Exact-string replacement; fails if <code>old_string</code> is missing or not unique.</td></tr>
        <tr><td><code>BatchEditTool</code></td><td>Write</td><td>Apply many edits in one call; aborts atomically on any failure.</td></tr>
        <tr><td><code>ApplyPatchTool</code></td><td>Write</td><td>Apply a unified diff patch.</td></tr>
      </tbody>
    </table>

    <p>Write tools enforce read-before-write: a file must have been read in the current session before it can be modified, preventing blind overwrites.</p>

    <h2>Shell execution</h2>

    <table>
      <thead><tr><th>Tool</th><th>Purpose</th></tr></thead>
      <tbody>
        <tr><td><code>BashTool</code></td><td>Execute shell commands with persistent working directory and environment.</td></tr>
        <tr><td><code>MonitorTool</code></td><td>Tail a long-running background process started by Bash.</td></tr>
        <tr><td><code>PtyBashTool</code></td><td>Bash in a pseudo-terminal — for commands needing TTY.</td></tr>
        <tr><td><code>PowerShellTool</code></td><td>PowerShell execution on Windows.</td></tr>
        <tr><td><code>ReplTool</code></td><td>Persistent REPL session (python, node, etc.).</td></tr>
      </tbody>
    </table>

    <h2>Search</h2>

    <table>
      <thead><tr><th>Tool</th><th>Purpose</th></tr></thead>
      <tbody>
        <tr><td><code>GlobTool</code></td><td>Match files by glob pattern (e.g. <code>src/**/*.rs</code>).</td></tr>
        <tr><td><code>GrepTool</code></td><td>Regex-search file contents — ripgrep semantics.</td></tr>
        <tr><td><code>ToolSearchTool</code></td><td>Look up tools by name/keyword (used by the model for deferred tools).</td></tr>
      </tbody>
    </table>

    <h2>Web</h2>

    <table>
      <thead><tr><th>Tool</th><th>Purpose</th></tr></thead>
      <tbody>
        <tr><td><code>WebFetchTool</code></td><td>Fetch a URL and summarize/extract via a small fast model.</td></tr>
        <tr><td><code>WebSearchTool</code></td><td>Web search with snippet results.</td></tr>
      </tbody>
    </table>

    <h2>Task management</h2>

    <p><code>TaskCreate</code>, <code>TaskUpdate</code>, <code>TaskList</code>, <code>TaskGet</code>, <code>TaskOutput</code>, <code>TaskStop</code> — for breaking work into trackable units, visible to the user via <code>/tasks</code>.</p>

    <h2>Other categories</h2>

    <ul>
      <li><strong>Git</strong> — commit, branch, worktree</li>
      <li><strong>Notebooks</strong> — read and edit Jupyter notebooks</li>
      <li><strong>Desktop automation</strong> — screenshot, click, type (optional feature)</li>
      <li><strong>MCP tools</strong> — dynamically added when MCP servers connect; see <a href="#mcp">MCP</a></li>
    </ul>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/tools.md" target="_blank" rel="noopener">the full tools reference</a> for parameter schemas, return types, and per-tool quirks across all 40+ built-ins.</p>
  `;
}
