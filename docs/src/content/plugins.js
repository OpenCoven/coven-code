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

    <h2>plugin.toml example</h2>

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

    <h2>Required &amp; optional fields</h2>

    <table>
      <thead><tr><th>Field</th><th>Required</th><th>Purpose</th></tr></thead>
      <tbody>
        <tr><td><code>name</code></td><td>yes</td><td>Unique plugin identifier</td></tr>
        <tr><td><code>version</code></td><td>yes</td><td>Semver string</td></tr>
        <tr><td><code>description</code></td><td>no</td><td>One-line summary surfaced in <code>/plugin list</code></td></tr>
        <tr><td><code>author</code></td><td>no</td><td><code>{ name, email, url }</code></td></tr>
        <tr><td><code>commands</code> / <code>agents</code> / <code>skills</code></td><td>no</td><td>Extra files beyond the conventional directories</td></tr>
        <tr><td><code>mcp_servers</code></td><td>no</td><td>Inline MCP server definitions</td></tr>
        <tr><td><code>lsp_servers</code></td><td>no</td><td>Inline LSP server definitions</td></tr>
        <tr><td><code>hooks</code></td><td>no</td><td>Inline hook definitions</td></tr>
        <tr><td><code>user_config</code></td><td>no</td><td>Schema for user-configurable options</td></tr>
        <tr><td><code>capabilities</code></td><td>no</td><td>Capability grants; omit to allow all</td></tr>
        <tr><td><code>marketplace_id</code></td><td>no</td><td>ID for marketplace listings (<code>owner/name</code>)</td></tr>
      </tbody>
    </table>

    <h2>Managing plugins</h2>

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

    <h2>Inline hooks</h2>

    <p>Plugins can define hooks in the manifest. Each hook lists an event name and a command/prompt/agent/http handler. See <a href="#hooks">Hooks</a> for the full event list and handler types.</p>

    <pre><code data-lang="json">[[hooks]]
event   = "PreToolUse"
command = "./scripts/audit-tool.sh"
if      = "tool == 'BashTool'"</code></pre>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/plugins.md" target="_blank" rel="noopener">the full plugins reference</a> for the JSON manifest variant, marketplace metadata, and a complete plugin walkthrough.</p>
  `;
}
