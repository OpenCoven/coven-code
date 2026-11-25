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

    <p>When Coven Code saves the engine-global file, it preserves the JSON values of unknown top-level keys, including keys added by a newer version. This does not guarantee preservation of unknown keys inside <code>config</code> or other nested objects. Unknown keys from project settings are ignored and never copied into the global file.</p>

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

    <h3>Reactive compaction</h3>
    <p>With <code>COVEN_CODE_FEATURE_REACTIVE_COMPACT=1</code>, a streamed turn can summarize history older than the newest ten messages when input-token usage reaches 90% of the context window. At 97% usage emergency collapse takes priority, including for short conversations.</p>

    <h2>Next Appearance</h2>
    <p><code>nextAppearance</code> changes how the Next renderer (<code>COVEN_CODE_UI=next</code>) draws, and nothing else: no value reaches permissions, providers, identity or stored state. Classic ignores it. An invalid value falls back to its default and stays in the file, and so does any key this version does not know.</p>
    <table>
      <thead><tr><th>Key</th><th>Values</th><th>Default</th></tr></thead>
      <tbody>
        <tr><td><code>theme</code></td><td><code>auto</code>, <code>dark</code>, <code>light</code></td><td><code>auto</code></td></tr>
        <tr><td><code>colorMode</code></td><td><code>auto</code>, <code>truecolor</code>, <code>256</code>, <code>16</code>, <code>none</code></td><td><code>auto</code></td></tr>
        <tr><td><code>glyphs</code></td><td><code>auto</code>, <code>unicode</code>, <code>ascii</code></td><td><code>auto</code></td></tr>
        <tr><td><code>density</code></td><td><code>comfortable</code>, <code>compact</code></td><td><code>comfortable</code></td></tr>
        <tr><td><code>layout</code></td><td><code>focus</code>, <code>work</code>, <code>inspect</code></td><td><code>focus</code></td></tr>
        <tr><td><code>hints</code></td><td><code>true</code>, <code>false</code></td><td><code>false</code></td></tr>
      </tbody>
    </table>
    <p>The top-level <code>reduceMotion</code> setting makes Next's activity indicator static. The <code>work</code> and <code>inspect</code> layouts open the inspector only while the terminal is 120x32 or larger.</p>
    <p>Under Next, <code>/config appearance</code> (or <code>/settings appearance</code>) lists these settings. <code>/config appearance &lt;setting&gt; &lt;value&gt;</code> previews one at once without writing anything; <code>save</code> keeps the preview, <code>revert</code> drops it, and <code>reset</code> previews the defaults for the six <code>nextAppearance</code> settings and preserves the current <code>reduceMotion</code> choice, including an unsaved preview. A save changes only the settings you previewed; every other setting keeps its effect, and keys this version does not know are kept. A settings file that cannot be read as settings is refused and left alone.</p>
    <p>Project values for the six <code>nextAppearance</code> settings take precedence inside that folder. On load, reduced motion is enabled if either your user settings or the project settings enable it. A project value of <code>false</code> leaves a user value of <code>true</code> in effect.</p>

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
