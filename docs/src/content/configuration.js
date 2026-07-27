export const meta = { title: 'Configuration' };

export function render() {
  return `
    <h1>Configuration</h1>
    <p class="lead">Coven Code is configured through a layered system of JSON files, environment variables, and command-line flags. Project settings override global settings; CLI flags override both.</p>

    <h2>File Locations</h2>

    <p>The global settings file lives at:</p>

    <pre><code data-lang="bash">~/.coven-code/settings.json</code></pre>

    <p>The directory is created automatically on first run. Files are standard JSON (or JSONC — comments are stripped before parsing).</p>

    <h3>Per-Project Settings</h3>

    <p>Coven Code walks up from the current working directory looking for a project-level settings file. The first file found wins:</p>

    <pre><code data-lang="bash">&lt;project-root&gt;/.coven-code/settings.json
&lt;project-root&gt;/.coven-code/settings.jsonc</code></pre>

    <p>Keys present in the project file override the global value; keys absent fall back to global.</p>

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
    <p>Set <code>"permission_mode"</code> to one of <code>"default"</code>, <code>"acceptEdits"</code>, <code>"bypassPermissions"</code>, or <code>"plan"</code>. See the <a href="#tools">interactive permission visualizer</a> for what each mode allows, prompts on, and blocks.</p>

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
        <tr><td><code>OPENAI_API_KEY</code></td><td>API key for OpenAI provider.</td></tr>
        <tr><td><code>COVEN_CODE_HOME</code></td><td>Override the default <code>~/.coven-code/</code> directory.</td></tr>
      </tbody>
    </table>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/configuration.md" target="_blank" rel="noopener">the full configuration reference</a> for every option, including <code>agents</code>, <code>commands</code>, <code>permissionRules</code>, and per-provider keys.</p>
  `;
}
