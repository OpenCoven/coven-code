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
    <table>
      <thead><tr><th>Key</th><th>Default</th><th>Description</th></tr></thead>
      <tbody>
        <tr><td><code>api_key</code></td><td><code>null</code></td><td>Anthropic API key. Overrides <code>ANTHROPIC_API_KEY</code>. Prefer the env var in shared environments.</td></tr>
        <tr><td><code>model</code></td><td>provider default</td><td>Model ID. Falls back to the provider's default (e.g. <code>claude-sonnet-4-6</code>).</td></tr>
        <tr><td><code>max_tokens</code></td><td><code>8192</code></td><td>Maximum tokens per model response.</td></tr>
        <tr><td><code>provider</code></td><td><code>"anthropic"</code></td><td>Active provider.</td></tr>
      </tbody>
    </table>

    <h3>Permission Mode</h3>
    <table>
      <thead><tr><th>Mode</th><th>Behavior</th></tr></thead>
      <tbody>
        <tr><td><code>default</code></td><td>Prompt for any tool that touches your filesystem or shell.</td></tr>
        <tr><td><code>acceptEdits</code></td><td>Auto-approve file edits; still prompt for shell.</td></tr>
        <tr><td><code>bypassPermissions</code></td><td>No prompts. Use only in trusted, sandboxed contexts.</td></tr>
        <tr><td><code>plan</code></td><td>Read-only mode. Model can read and search but cannot write or run commands.</td></tr>
      </tbody>
    </table>

    <h3>Familiar</h3>
    <p>Set <code>"familiar"</code> to the id of your active familiar (e.g. <code>"kitty"</code>, <code>"raven"</code>). This drives the welcome-screen portrait and the <code>/agents</code> overlay when the daemon is online.</p>

    <pre><code data-lang="json">{
  "familiar": "raven",
  "config": {
    "model": "claude-opus-4-7",
    "permission_mode": "default",
    "auto_compact": true,
    "compact_threshold": 0.8
  }
}</code></pre>

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
