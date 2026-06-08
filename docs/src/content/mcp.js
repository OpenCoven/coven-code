export const meta = { title: 'MCP' };

export function render() {
  return `
    <h1>Model Context Protocol</h1>
    <p class="lead">MCP servers extend Coven Code with external tools, resources, and prompts. Servers run as subprocesses (stdio) or remote HTTP/SSE endpoints; their capabilities are discovered at handshake and wrapped as native tools the model can call.</p>

    <h2>What MCP offers</h2>

    <ul>
      <li><strong>Tools</strong> — callable functions the model can invoke (analogous to built-ins like <code>Bash</code> or <code>Read</code>)</li>
      <li><strong>Resources</strong> — URI-addressable data sources the model can read</li>
      <li><strong>Prompts</strong> — reusable prompt templates the server exposes</li>
    </ul>

    <h2>Transports</h2>

    <h3>stdio (subprocess)</h3>

    <p>Default transport. Coven Code spawns the server as a child process and communicates over its stdin/stdout using newline-delimited JSON-RPC 2.0.</p>

    <pre><code data-lang="json">{
  "name": "filesystem",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user/projects"],
  "type": "stdio"
}</code></pre>

    <h3>HTTP / SSE</h3>

    <p>For servers running as standalone HTTP services:</p>

    <pre><code data-lang="json">{
  "name": "remote-tools",
  "url": "https://mcp.example.com/sse",
  "type": "http"
}</code></pre>

    <h2>Configuration fields</h2>

    <table>
      <thead><tr><th>Field</th><th>Required</th><th>Description</th></tr></thead>
      <tbody>
        <tr><td><code>name</code></td><td>yes</td><td>Unique identifier for the server</td></tr>
        <tr><td><code>command</code></td><td>stdio only</td><td>Executable (e.g. <code>"npx"</code>, <code>"uvx"</code>)</td></tr>
        <tr><td><code>args</code></td><td>no</td><td>Arguments passed to <code>command</code></td></tr>
        <tr><td><code>env</code></td><td>no</td><td>Extra env vars for the child process</td></tr>
        <tr><td><code>url</code></td><td>http only</td><td>Full SSE endpoint URL</td></tr>
        <tr><td><code>type</code></td><td>no</td><td><code>"stdio"</code> (default) or <code>"http"</code></td></tr>
      </tbody>
    </table>

    <h2>Environment expansion</h2>

    <p>All string fields support shell-style variable expansion before the server is launched:</p>

    <table>
      <thead><tr><th>Pattern</th><th>Behaviour</th></tr></thead>
      <tbody>
        <tr><td><code>\${VAR}</code></td><td>Substituted with <code>VAR</code> from env; left unchanged if unset</td></tr>
        <tr><td><code>\${VAR:-default}</code></td><td>Substituted with <code>VAR</code> if set; falls back to <code>default</code></td></tr>
      </tbody>
    </table>

    <h2>Adding servers to settings.json</h2>

    <pre><code data-lang="json">{
  "config": {
    "mcp_servers": [
      {
        "name": "filesystem",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "\${HOME}/projects"],
        "type": "stdio"
      },
      {
        "name": "github",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-github"],
        "env": {
          "GITHUB_PERSONAL_ACCESS_TOKEN": "\${GITHUB_TOKEN}"
        }
      },
      {
        "name": "remote-api",
        "url": "https://mcp.example.com/sse",
        "type": "http"
      }
    ]
  }
}</code></pre>

    <p>Project-level servers go in <code>.coven-code/settings.json</code> at the project root and override global servers with the same name. Plugin-provided servers are merged before the initial MCP connection, so they're available on first startup.</p>

    <h2>The /mcp command</h2>

    <p>Use <code>/mcp</code> inside an interactive session to list connected servers, see their tools/resources/prompts, reconnect, or inspect connection errors.</p>

    <h2>MCP tools available to the model</h2>

    <ul>
      <li><code>ListMcpResources</code> — enumerate URIs exposed by a server</li>
      <li><code>ReadMcpResource</code> — fetch a specific resource</li>
    </ul>

    <p>Reconnection uses exponential backoff; transient failures don't require a restart.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/mcp.md" target="_blank" rel="noopener">the full MCP reference</a> for popular servers (filesystem, github, slack, postgres, brave-search), Python-based servers via <code>uvx</code>, and complete config examples.</p>
  `;
}
