export const meta = { title: 'Configuration' };

export function render() {
  return `
    <h1>Configuration</h1>
    <p class="lead">Coven Code is configured through a layered system of JSON files, environment variables, and command-line flags. Safe project fields merge with global settings; CLI overrides apply afterward. Merging is field-specific.</p>

    <h2>File Locations</h2>

    <p>The global settings file lives at:</p>

    <pre><code data-lang="bash">&lt;engine-home&gt;/settings.json</code></pre>

    <p>The engine home is <code>~/.coven/code</code> when a Coven home exists, otherwise <code>~/.coven-code</code>. <code>COVEN_CODE_HOME</code> overrides it. Global settings require plain JSON without comments. Shared Coven settings supply only model, theme, and permission-mode defaults.</p>

    <h3>Per-Project Settings</h3>

    <p>Coven Code walks up from the current working directory looking for a project-level settings file. The first file found wins:</p>

    <pre><code data-lang="bash">&lt;project-root&gt;/.coven-code/settings.json
&lt;project-root&gt;/.coven-code/settings.jsonc</code></pre>

    <p>Project files accept comments. An invalid or unreadable first file stops the search. Provider routing and credentials, MCP/LSP definitions, hooks, hosted policy, permission rules, and workspace read roots are stripped before merging. Some omitted project fields reset global values to defaults; consult the full reference before relying on inheritance.</p>

    <h2>Top-Level Structure</h2>

    <pre><code data-lang="json">{
  "version": 1,
  "provider": "anthropic",
  "config": { },
  "providers": { },
  "projects": { },
  "commands": { },
  "formatter": { },
  "agents": { },
  "skills": { },
  "permissionRules": [],
  "enabledPlugins": [],
  "disabledPlugins": [],
  "hasCompletedOnboarding": false
}</code></pre>

    <p>Most day-to-day options live inside the <code>config</code> object. Provider credentials live in the <code>providers</code> map.</p>

    <h2>Common <code>config</code> Options</h2>

    <h3>Model and Tokens</h3>
    <div class="fields-grid">
      <div class="field-card">
        <div class="field-card-head">
          <span class="field-card-name">api_key</span>
          <span class="field-card-meta">null</span>
        </div>
        <div class="field-card-desc">Anthropic API key. Overrides <code>ANTHROPIC_API_KEY</code>. Prefer the env var in shared environments.</div>
      </div>
      <div class="field-card">
        <div class="field-card-head">
          <span class="field-card-name">model</span>
          <span class="field-card-meta">provider default</span>
        </div>
        <div class="field-card-desc">Model ID. Falls back to the provider's default (e.g. <code>claude-sonnet-4-6</code>).</div>
      </div>
      <div class="field-card">
        <div class="field-card-head">
          <span class="field-card-name">max_tokens</span>
          <span class="field-card-meta">8192</span>
        </div>
        <div class="field-card-desc">Maximum tokens per model response.</div>
      </div>
      <div class="field-card">
        <div class="field-card-head">
          <span class="field-card-name">provider</span>
          <span class="field-card-meta">"anthropic"</span>
        </div>
        <div class="field-card-desc">Active provider. See <a href="#providers">Providers</a> for the full list.</div>
      </div>
    </div>

    <h3>Permission Mode</h3>
    <p>Set <code>"permission_mode"</code> to one of <code>"default"</code>, <code>"acceptEdits"</code>, <code>"bypassPermissions"</code>, or <code>"plan"</code>. <code>acceptEdits</code> auto-approves only <code>Edit</code>; shell commands retain normal checks. Rules and tool-specific guards also apply. CLI values use <code>accept-edits</code> and <code>bypass-permissions</code>.</p>

    <h3>Familiar</h3>
    <p>Set <code>"familiar"</code> to the id of your active familiar (e.g. <code>"kitty"</code>, <code>"raven"</code>). This drives the welcome-screen portrait and the <code>/familiar</code> overlay when the daemon is online.</p>

    <pre><code data-lang="json">{
  "familiar": "raven",
  "config": {
    "model": "claude-opus-4-7",
    "permission_mode": "default",
    "auto_compact": true,
    "compact_threshold": 0.8
  }
}</code></pre>

    <h2>Memory Retention</h2>
    <p>AGENTS.md frontmatter supports lifecycle fields for hosted review memory. <code>expires_at</code> uses <code>YYYY-MM-DD</code> and always wins over retention defaults. <code>retention_class</code> can be <code>standard</code> (no automatic expiry), <code>short_lived</code> (30 days from <code>created_at</code>), <code>security</code> (90 days), or <code>legal_hold</code> (no automatic expiry and requires <code>--force</code> for operator expiry/deletion).</p>
    <p>Operators can run <code>coven-code memory list</code>, <code>expire</code>, <code>redact</code>, <code>delete</code>, <code>conflicts</code>, <code>resolve-conflict</code>, and <code>ledger --json</code>. Redaction and deletion write tombstone stubs; the audit ledger exports ids, timestamps, reasons, and provenance without original removed content.</p>

    <h2>Environment Variables</h2>

    <table>
      <thead><tr><th>Variable</th><th>Purpose</th></tr></thead>
      <tbody>
        <tr><td><code>ANTHROPIC_API_KEY</code></td><td>API key for Anthropic provider.</td></tr>
        <tr><td><code>COVEN_HOME</code></td><td>Relocate the shared Coven home.</td></tr>
        <tr><td><code>COVEN_CODE_HOME</code></td><td>Explicitly override the engine home.</td></tr>
      </tbody>
    </table>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/configuration.md" target="_blank" rel="noopener">the full configuration reference</a> for every option, including <code>agents</code>, <code>commands</code>, <code>permissionRules</code>, and per-provider keys.</p>
  `;
}
