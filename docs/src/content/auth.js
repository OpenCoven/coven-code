export const meta = { title: 'Authentication' };

export function render() {
  return `
    <h1>Authentication</h1>
    <p class="lead">Coven Code supports API keys, OAuth (Claude.ai / Console / ChatGPT), and per-provider environment variables, with named multi-account profiles so you can switch identities without re-logging-in.</p>

    <h2>Credential priority</h2>

    <p>For Anthropic, credentials are checked in this order — the first non-empty match wins:</p>

    <ol>
      <li><code>--api-key</code> flag (session-only)</li>
      <li><code>api_key</code> in <code>~/.coven-code/settings.json</code></li>
      <li><code>ANTHROPIC_API_KEY</code> environment variable</li>
      <li>Tokens for the active Anthropic profile under <code>~/.coven-code/accounts/anthropic/&lt;id&gt;/oauth_tokens.json</code></li>
      <li>Legacy <code>~/.coven-code/oauth_tokens.json</code> (auto-migrated on first read)</li>
    </ol>

    <p>Provider-specific credentials (OpenAI, Google, etc.) follow the same pattern with their own env vars. Codex (ChatGPT subscription) accounts live under <code>~/.coven-code/accounts/codex/&lt;id&gt;/</code>.</p>

    <h2>Method 1: API key</h2>

    <p>Get a key at <a href="https://console.anthropic.com" target="_blank" rel="noopener">console.anthropic.com</a> → Settings → API Keys, then either set it in your shell:</p>

    <pre><code data-lang="bash">export ANTHROPIC_API_KEY="sk-ant-api03-..."</code></pre>

    <p>or store it in <code>~/.coven-code/settings.json</code> (restrict file permissions on shared systems):</p>

    <pre><code data-lang="json">{
  "config": {
    "api_key": "sk-ant-api03-..."
  }
}</code></pre>

    <p>or pass it per-invocation:</p>

    <pre><code data-lang="bash">coven-code --api-key "sk-ant-..." "your prompt"</code></pre>

    <h2>Method 2: OAuth login</h2>

    <p>OAuth 2.0 PKCE flow through Claude.ai or Console. Requires a registered first-party client ID:</p>

    <pre><code data-lang="bash">export COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID=&lt;registered-client-id&gt;
coven-code auth login</code></pre>

    <p>A localhost callback server starts, your browser opens the authorization URL, and the tokens are saved under <code>~/.coven-code/accounts/anthropic/&lt;profile-id&gt;/</code>. Use <code>--codex</code> to authenticate against ChatGPT subscription credentials instead.</p>

    <h2>Multi-account profiles</h2>

    <pre><code data-lang="bash"># Add accounts (each login becomes its own profile)
coven-code auth login --label work
coven-code auth login --codex --label personal

# Inspect
coven-code auth list

# Switch the active account
coven-code auth switch work
coven-code auth switch --codex personal

# Remove a stored profile
coven-code auth remove old-account

# Logout (clears tokens for the active profile)
coven-code auth logout</code></pre>

    <p>Inside the TUI, the same operations are available as slash commands:</p>

    <table>
      <thead><tr><th>Command</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><code>/login</code></td><td>OAuth login (use <code>--codex</code> for ChatGPT, <code>--label &lt;name&gt;</code> to name the profile)</td></tr>
        <tr><td><code>/accounts</code></td><td>List stored Anthropic + Codex accounts</td></tr>
        <tr><td><code>/switch &lt;id&gt;</code></td><td>Switch active account (<code>--codex</code> for Codex)</td></tr>
        <tr><td><code>/logout</code></td><td>Clear credentials for the active profile (<code>--all</code> to purge)</td></tr>
      </tbody>
    </table>

    <h2>Token storage</h2>

    <p>Each profile gets its own directory:</p>

    <pre><code data-lang="bash">~/.coven-code/accounts/
├── anthropic/
│   ├── work/oauth_tokens.json
│   └── personal/oauth_tokens.json
├── codex/
│   └── default/oauth_tokens.json
└── accounts.json    # registry of profiles + active selection</code></pre>

    <p>Identity is detected from the OAuth JWT, so re-logging-in the same account is idempotent — it updates the existing profile rather than creating a duplicate.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/auth.md" target="_blank" rel="noopener">the full authentication reference</a> for the device-code flow, manual callback fallback, identity detection internals, and the per-provider credential store.</p>
  `;
}
