export const meta = { title: 'Plugins' };

export function render() {
  return `
    <h1>Plugins</h1>
    <p class="lead">Plugins extend Coven Code with custom slash commands, agents, skills, MCP servers, LSP servers, hooks, and output styles. They are declarative TOML or JSON manifests with optional supporting files — no compilation required.</p>

    <h2>Discovery</h2>

    <p>Plugins live in <code>~/.coven-code/plugins/</code>. Any subdirectory containing a valid <code>plugin.toml</code> or <code>plugin.json</code> is loaded on startup.</p>

    <pre class="file-tree">~/.coven-code/plugins/
├── my-plugin/
│   ├── plugin.toml          ← manifest
│   ├── commands/            ← *.md slash command definitions
│   ├── agents/              ← *.md agent definitions
│   ├── skills/              ← subdirectories with SKILL.md
│   ├── hooks/               ← hooks.json (optional)
│   └── output-styles/       ← *.md or *.json style definitions
└── another-plugin/
    └── plugin.json</pre>

    <p>The loader normalises camelCase and snake_case field names, so manifests written in either convention are accepted.</p>

    <h2>plugin.toml Example</h2>

    <pre><code data-lang="json">name        = "my-plugin"
version     = "1.0.0"
description = "Adds custom commands and hooks for my workflow"
license     = "MIT"
keywords    = ["formatting", "git"]

[author]
name  = "Your Name"
email = "you@example.com"

# Inline MCP server definitions
[[mcp_servers]]
name    = "my-tool-server"
command = "npx"
args    = ["-y", "my-mcp-server"]
type    = "stdio"

[mcp_servers.env]
API_TOKEN = "\${MY_SERVICE_TOKEN}"

# Inline LSP server definitions
[[lsp_servers]]
name    = "pyright"
command = "pyright-langserver"
args    = ["--stdio"]
transport = "stdio"

# User-configurable options (surfaced in /plugin info)
[user_config.api_token]
type        = "string"
title       = "API Token"
required    = true
sensitive   = true

# Capability grants (omit to allow all)
capabilities = ["read_files", "network", "shell"]

# Marketplace identifier
marketplace_id = "you/my-plugin"</code></pre>

    <h2>Manifest Fields</h2>

    <p>Type to filter by field name or purpose. Click a chip to scope to a group — Identity (required), Metadata, Content, Inline definitions, or user-facing Config.</p>

    <div class="demo" x-data="pluginFieldExplorer">
      <div class="demo-header">
        <span>plugin field explorer · <span x-text="count"></span> / <span x-text="total"></span> shown</span>
      </div>
      <div class="demo-body">
        <div class="explorer-controls">
          <input
            type="text"
            class="explorer-input"
            placeholder="Search fields — try 'name', 'mcp', 'required', 'marketplace'…"
            x-model="query"
            aria-label="Search plugin manifest fields"
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

    <h2>Managing Plugins</h2>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/plugin list</code></td><td>List installed plugins and their state</td></tr>
        <tr><td><code>/plugin info &lt;name&gt;</code></td><td>Show manifest, capabilities, user config</td></tr>
        <tr><td><code>/plugin enable &lt;name&gt;</code></td><td>Enable a plugin</td></tr>
        <tr><td><code>/plugin disable &lt;name&gt;</code></td><td>Disable without uninstalling</td></tr>
        <tr><td><code>/reload-plugins</code></td><td>Re-scan the plugins directory</td></tr>
      </tbody>
    </table>

    <p>Enabled/disabled state is persisted in <code>settings.json</code> under <code>enabledPlugins</code> and <code>disabledPlugins</code>.</p>

    <h2>Inline Hooks</h2>

    <p>Plugins can define hooks in the manifest. Each hook lists an event name and a command/prompt/agent/http handler. See <a href="#hooks">Hooks</a> for the full event list and handler types.</p>

    <pre><code data-lang="json">[[hooks]]
event   = "PreToolUse"
command = "./scripts/audit-tool.sh"
if      = "tool == 'BashTool'"</code></pre>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/plugins.md" target="_blank" rel="noopener">the full plugins reference</a> for the JSON manifest variant, marketplace metadata, and a complete plugin walkthrough.</p>
  `;
}
