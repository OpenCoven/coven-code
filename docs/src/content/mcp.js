export const meta = { title: 'MCP' };

export function render() {
  return `
    <h1>Model Context Protocol</h1>
    <p class="lead">MCP servers extend Coven Code with external tools, resources, and prompts. Servers run as subprocesses (stdio) or remote HTTP/SSE endpoints; their capabilities are discovered at handshake and wrapped as native tools the model can call.</p>

    <h2>What MCP Offers</h2>

    <ul>
      <li><strong>Tools</strong> — callable functions the model can invoke (analogous to built-ins like <code>Bash</code> or <code>Read</code>)</li>
      <li><strong>Resources</strong> — URI-addressable data sources the model can read</li>
      <li><strong>Prompts</strong> — reusable prompt templates the server exposes</li>
    </ul>

    <h2>Transports</h2>

    <h3>stdio (Subprocess)</h3>

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

    <h2>Configuration Fields</h2>

    <p>Type to filter, or pick a chip to scope. <em>Required-ness depends on transport</em> — <code>command</code> is required for stdio, <code>url</code> is required for http.</p>

    <div class="demo" x-data="mcpFieldExplorer">
      <div class="demo-header">
        <span>mcp config explorer · <span x-text="count"></span> / <span x-text="total"></span> shown</span>
      </div>
      <div class="demo-body">
        <div class="explorer-controls">
          <input
            type="text"
            class="explorer-input"
            placeholder="Search fields — try 'stdio', 'env', 'required'…"
            x-model="query"
            aria-label="Search MCP config fields"
          />
          <span class="explorer-count">
            <span x-text="count"></span> matches
          </span>
        </div>
        <div class="explorer-chips">
          <template x-for="cat in categories" :key="cat">
            <button
              type="button"
              class="explorer-chip"
              :aria-pressed="category === cat"
              @click="pick(cat)"
            >
              <span x-text="cat"></span>
              <span class="explorer-chip-count" x-text="countIn(cat)"></span>
            </button>
          </template>
          <button
            type="button"
            class="explorer-clear"
            x-show="query || category"
            @click="clear()"
          >Clear</button>
        </div>
        <div class="explorer-results" x-show="count > 0">
          <template x-for="item in filtered" :key="item.id">
            <div class="explorer-item">
              <div class="explorer-item-head">
                <span class="explorer-item-id" x-text="item.id"></span>
                <span class="explorer-item-cat" x-text="item.category"></span>
              </div>
              <div class="explorer-item-desc" x-text="item.desc"></div>
            </div>
          </template>
        </div>
        <div class="explorer-empty" x-show="count === 0">
          No fields match. <a href="#" @click.prevent="clear()" style="color: var(--color-accent);">Clear filters</a>
        </div>
      </div>
    </div>

    <h2>Environment Expansion</h2>

    <p>All string fields support shell-style variable expansion before the server is launched:</p>

    <table>
      <thead><tr><th>Pattern</th><th>Behaviour</th></tr></thead>
      <tbody>
        <tr><td><code>\${VAR}</code></td><td>Substituted with <code>VAR</code> from env; left unchanged if unset</td></tr>
        <tr><td><code>\${VAR:-default}</code></td><td>Substituted with <code>VAR</code> if set; falls back to <code>default</code></td></tr>
      </tbody>
    </table>

    <h2>Adding Servers to settings.json</h2>

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

    <h2>The /mcp Command</h2>

    <p>Use <code>/mcp</code> inside an interactive session to list connected servers, see their tools/resources/prompts, reconnect, or inspect connection errors.</p>

    <h2>MCP Tools Available to the Model</h2>

    <ul>
      <li><code>ListMcpResources</code> — enumerate URIs exposed by a server</li>
      <li><code>ReadMcpResource</code> — fetch a specific resource</li>
    </ul>

    <p>Reconnection uses exponential backoff; transient failures don't require a restart.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/mcp.md" target="_blank" rel="noopener">the full MCP reference</a> for popular servers (filesystem, github, slack, postgres, brave-search), Python-based servers via <code>uvx</code>, and complete config examples.</p>
  `;
}
