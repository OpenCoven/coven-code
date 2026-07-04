export const meta = { title: 'Authentication' };

export function render() {
  return `
    <h1>Authentication</h1>
    <p class="lead">Coven Code supports API keys, OAuth (Claude.ai / Console / ChatGPT), and per-provider environment variables, with named multi-account profiles so you can switch identities without re-logging-in.</p>

    <h2>Credential Priority</h2>

    <p>For Anthropic, credentials are checked in this order — the first non-empty match wins:</p>

    <ol>
      <li><code>--api-key</code> flag (session-only)</li>
      <li><code>api_key</code> in <code>~/.coven-code/settings.json</code></li>
      <li><code>ANTHROPIC_API_KEY</code> environment variable</li>
      <li>Tokens for the active Anthropic profile under <code>~/.coven-code/accounts/anthropic/&lt;id&gt;/oauth_tokens.json</code></li>
      <li>Legacy <code>~/.coven-code/oauth_tokens.json</code> (auto-migrated on first read)</li>
    </ol>

    <p>Provider-specific credentials (OpenAI, Google, etc.) follow the same pattern with their own env vars. Codex (ChatGPT subscription) accounts live under <code>~/.coven-code/accounts/codex/&lt;id&gt;/</code>.</p>

    <h2>Method 1: API Key</h2>

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

    <h2>Method 2: OAuth Login</h2>

    <p>OAuth 2.0 PKCE flow through Claude.ai or Console. Requires a registered first-party client ID:</p>

    <pre><code data-lang="bash">export COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID=&lt;registered-client-id&gt;
coven-code auth login</code></pre>

    <p>A localhost callback server starts, your browser opens the authorization URL, and the tokens are saved under <code>~/.coven-code/accounts/anthropic/&lt;profile-id&gt;/</code>. If you already use Claude Code or <code>ant</code>, open <code>/connect</code> and choose Claude CLI to import that local login instead.</p>

    <h2>Method 3: Codex Login</h2>

    <p>Codex uses ChatGPT/Codex browser OAuth and stores profiles under <code>~/.coven-code/accounts/codex/&lt;id&gt;/</code>:</p>

    <pre><code data-lang="bash">coven-code codex login
coven-code --provider codex</code></pre>

    <h2>Multi-Account Profiles</h2>

    <pre><code data-lang="bash"># Add accounts (each login becomes its own profile)
coven-code auth login --label work
coven-code codex login --label personal

# Inspect
coven-code auth list
coven-code codex list

# Switch the active account
coven-code auth switch work
coven-code codex switch personal

# Remove a stored profile
coven-code auth remove old-account
coven-code codex remove old-account

# Logout (clears tokens for the active profile)
coven-code auth logout
coven-code codex logout</code></pre>

    <p>Inside the TUI, the same operations are available as slash commands:</p>

    <div class="fields-grid">
      <div class="field-card">
        <div class="field-card-name">/login</div>
        <div class="field-card-desc">OAuth login for Anthropic. Use <code>--label &lt;name&gt;</code> to name the profile.</div>
      </div>
      <div class="field-card">
        <div class="field-card-name">/accounts</div>
        <div class="field-card-desc">List stored Anthropic + Codex accounts.</div>
      </div>
      <div class="field-card">
        <div class="field-card-name">/login switch &lt;id&gt;</div>
        <div class="field-card-desc">Switch active account. Use <code>--codex</code> for Codex.</div>
      </div>
      <div class="field-card">
        <div class="field-card-name">/logout</div>
        <div class="field-card-desc">Clear credentials for the active profile. <code>--all</code> to purge.</div>
      </div>
    </div>

    <h2>Token Storage</h2>

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
