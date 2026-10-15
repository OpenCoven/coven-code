export const meta = { title: 'Providers' };

export function render() {
  return `
    <h1>LLM Providers</h1>
    <p class="lead">Coven Code supports exactly two providers: Anthropic (Claude) and Codex (OpenAI Codex through ChatGPT OAuth).</p>

    <h2>Selecting a Provider</h2>

    <p>Use <code>--provider</code> on any invocation to override the active provider:</p>

    <pre><code data-lang="bash">coven-code --provider anthropic "refactor this module"
coven-code --provider codex "explain this function"</code></pre>

    <p>Set <code>provider</code> in the engine-global <code>settings.json</code> to persist selection. The default home is <code>~/.coven-code</code> for standalone use or <code>~/.coven/code</code> under Coven; <code>COVEN_CODE_HOME</code> overrides it.</p>

    <pre><code data-lang="json">{
  "provider": "anthropic"
}</code></pre>

    <p>When no provider is specified, Coven Code defaults to <strong>Anthropic</strong>.</p>

    <h2>Browse Providers</h2>

    <p>Search the two supported provider identities by name, authentication method, or model family.</p>

    <div class="demo" x-data="providerExplorer">
      <div class="demo-header">
        <span>provider explorer · <span x-text="count"></span> / <span x-text="total"></span> shown</span>
      </div>
      <div class="demo-body">
        <div class="explorer-controls">
          <input
            type="text"
            class="explorer-input"
            placeholder="Search providers — try 'Claude', 'Codex', 'OAuth', 'API_KEY'…"
            x-model="query"
            aria-label="Search providers"
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
                <span class="explorer-item-id" x-html="mark(item.id)"></span>
                <span class="explorer-item-cat" x-html="mark(item.category)"></span>
              </div>
              <div class="explorer-item-desc" x-html="mark(item.desc)"></div>
            </div>
          </template>
        </div>
        <div class="explorer-empty" x-show="count === 0">
          No providers match. <a href="#" @click.prevent="clear()" style="color: var(--color-accent);">Clear filters</a>
        </div>
      </div>
    </div>

    <h2>Anthropic (Default)</h2>

    <p>With an Anthropic API key, requests use the Messages API. Without a key, turns run through the local <code>claude</code> CLI, which owns its authentication. Coven Code does not import or replay its OAuth tokens. Set <code>ANTHROPIC_API_KEY</code> or use <code>/account connect</code> for setup.</p>

    <p>The separate direct Anthropic OAuth flow requires a configured <code>COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID</code>. It is not a Claude CLI credential-import path.</p>

    <h2>Codex</h2>
    <p>Sign in with ChatGPT OAuth using the engine's browser login. This provider does not use a standalone OpenAI API key.</p>
    <pre><code data-lang="bash">coven-code codex login
coven-code --provider codex</code></pre>

    <h2>Model Catalog</h2>
    <p>Model defaults and availability depend on the installed catalog and account. Inspect the resolved catalog instead of relying on a static pricing table; an entry does not guarantee subscription entitlement.</p>
    <pre><code data-lang="bash">coven-code models anthropic
coven-code models codex --json</code></pre>

    <h2>Per-Provider Configuration</h2>

    <p>Provider-specific settings live under <code>providers.&lt;id&gt;</code> in <code>settings.json</code>:</p>

    <pre><code data-lang="json">{
  "provider": "anthropic",
  "providers": {
    "anthropic": {
      "api_key": "sk-ant-...",
      "models_whitelist": ["claude-sonnet-4-6", "claude-haiku-4-5-20251001"]
    }
  }
}</code></pre>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/providers.md" target="_blank" rel="noopener">the full providers reference</a> for configuration fields and model catalog behavior.</p>
  `;
}
