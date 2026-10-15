export const meta = { title: 'Authentication' };

export function render() {
  return `
    <h1>Authentication</h1>
    <p class="lead">Claude uses an Anthropic API key or the authenticated local Claude CLI. Codex uses ChatGPT OAuth. Coven Code does not import or replay another CLI's OAuth tokens.</p>

    <h2>Claude API Key</h2>
    <pre><code data-lang="bash">export ANTHROPIC_API_KEY="&lt;your-key&gt;"
coven-code --provider anthropic</code></pre>
    <p>Inject keys through your secret-management mechanism for CI. Engine-global settings can also store a key under <code>config.api_key</code>. Avoid literal keys in committed files, copied diagnostics, or command history.</p>

    <h2>Local Claude CLI</h2>
    <p>Without an API key, ordinary Claude turns run through the local <code>claude</code> executable. Authenticate inside that CLI and make it available on PATH. Its login stays there; <code>coven-code auth import</code> has been removed. Engine auth status does not prove whether that CLI has a usable login.</p>

    <h2>Engine-Owned Anthropic OAuth</h2>
    <p>The separate direct OAuth flow requires a client identity registered for Coven Code. Do not reuse another CLI's client ID.</p>
    <pre><code data-lang="bash">export COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID="&lt;registered-client-id&gt;"
coven-code auth login --label work
# Console variant creates an API key:
coven-code auth login --console --label work</code></pre>
    <p>Follow the browser URL and fallback instructions printed by the flow. Stored bearer tokens must match the configured engine-owned client identity.</p>

    <h2>Codex Browser Login</h2>
    <pre><code data-lang="bash">coven-code codex login --label personal
coven-code --provider codex</code></pre>
    <p>This is ChatGPT OAuth, not a standalone OpenAI API-key path. The generic device-code helper in the source is not wired to a provider login command.</p>

    <h2>Account Management</h2>
    <pre><code data-lang="bash">coven-code auth list
coven-code codex list
coven-code accounts --json
coven-code auth switch &lt;profile-id&gt;
coven-code codex switch &lt;profile-id&gt;
coven-code auth remove &lt;profile-id&gt;
coven-code codex remove &lt;profile-id&gt;</code></pre>
    <p>Use the profile ID printed by login or list; a label does not guarantee a generated ID. Registry output can include identity metadata, so review it before sharing.</p>
    <p>In the TUI, use <code>/account connect</code>, <code>/account list</code>, <code>/account login</code>, <code>/account switch &lt;profile-id&gt;</code>, and <code>/account logout</code>. Use <code>--codex</code> with login, switch, or logout for Codex. <code>/account dedupe</code> reconciles duplicate profiles.</p>

    <h2>Status and Logout</h2>
    <pre><code data-lang="bash">coven-code auth status --json
coven-code codex status
coven-code auth logout
coven-code codex logout</code></pre>
    <p>Engine auth status returns JSON with a boolean <code>loggedIn</code> and exits 0 or 1. Codex status instead prints a human-readable active-profile summary; it does not expose the same JSON interface. Neither command proves live provider entitlement or successful inference.</p>
    <p>Logout affects engine-owned state. Environment keys, provider-specific settings, the separate credential store, and the local Claude CLI's login remain separate sources. Do not assume one logout purges every source.</p>

    <h2>Token Storage</h2>
    <p>The engine home normally is <code>~/.coven-code</code> standalone or <code>~/.coven/code</code> under Coven. Inspect <code>COVEN_CODE_HOME</code>, <code>COVEN_HOME</code>, and migration symlinks when locating your data. Relative to that home:</p>
    <pre><code data-lang="text">accounts.json
accounts/anthropic/&lt;id&gt;/oauth_tokens.json
accounts/codex/&lt;id&gt;/codex_tokens.json
auth.json
settings.json</code></pre>
    <p>The registry is beside the accounts directory. Keep credential files private and out of version control; never copy tokens from another CLI to populate them.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/auth.md" target="_blank" rel="noopener">the full authentication reference</a> for account commands, credential ownership, status limits, and troubleshooting.</p>
  `;
}
