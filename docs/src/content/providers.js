export const meta = { title: 'Providers' };

export function render() {
  return `
    <h1>LLM Providers</h1>
    <p class="lead">Coven Code supports a wide range of LLM providers through a unified <code>LlmProvider</code> trait. Switching between them requires only a configuration change.</p>

    <h2>Selecting a Provider</h2>

    <p>Use <code>--provider</code> on any invocation to override the active provider:</p>

    <pre><code data-lang="bash">coven-code --provider openai "refactor this module"
coven-code --provider ollama "explain this function"
coven-code --provider groq --model llama-3.3-70b-versatile "write tests"</code></pre>

    <p>Or set it persistently in <code>~/.coven-code/settings.json</code>:</p>

    <pre><code data-lang="json">{
  "provider": "openai"
}</code></pre>

    <p>When no provider is specified, Coven Code defaults to <strong>Anthropic</strong>.</p>

    <h2>Browse Providers</h2>

    <p>Type to filter by id, env var, or model family. Use the chips to narrow to cloud, aggregator, or local providers.</p>

    <div class="demo" x-data="providerExplorer">
      <div class="demo-header">
        <span>provider explorer · <span x-text="count"></span> / <span x-text="total"></span> shown</span>
      </div>
      <div class="demo-body">
        <div class="explorer-controls">
          <input
            type="text"
            class="explorer-input"
            placeholder="Search providers — try 'gemini', 'local', 'bedrock', 'API_KEY'…"
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
                <span class="explorer-item-id" x-text="item.id"></span>
                <span class="explorer-item-cat" x-text="item.category"></span>
              </div>
              <div class="explorer-item-desc" x-text="item.desc"></div>
            </div>
          </template>
        </div>
        <div class="explorer-empty" x-show="count === 0">
          No providers match. <a href="#" @click.prevent="clear()" style="color: var(--color-accent);">Clear filters</a>
        </div>
      </div>
    </div>

    <h2>Anthropic (Default)</h2>

    <p>Uses the <code>/v1/messages</code> streaming endpoint. Authenticate via <code>ANTHROPIC_API_KEY</code> or run <code>/login</code> for OAuth.</p>

    <table>
      <thead><tr><th>Model ID</th><th>Context</th><th>Max Output</th><th>Input ($/1M)</th><th>Output ($/1M)</th></tr></thead>
      <tbody>
        <tr><td><code>claude-opus-4-6</code></td><td>200,000</td><td>32,000</td><td>$15.00</td><td>$75.00</td></tr>
        <tr><td><code>claude-sonnet-4-6</code></td><td>200,000</td><td>16,000</td><td>$3.00</td><td>$15.00</td></tr>
        <tr><td><code>claude-haiku-4-5-20251001</code></td><td>200,000</td><td>8,096</td><td>$0.80</td><td>$4.00</td></tr>
      </tbody>
    </table>

    <p>All Anthropic models support tool calling, vision, and extended reasoning.</p>

    <h2>Per-Provider Configuration</h2>

    <p>Provider-specific settings live under <code>providers.&lt;id&gt;</code> in <code>settings.json</code>:</p>

    <pre><code data-lang="json">{
  "provider": "anthropic",
  "providers": {
    "anthropic": {
      "api_key": "sk-ant-...",
      "models_whitelist": ["claude-sonnet-4-6", "claude-haiku-4-5-20251001"]
    },
    "ollama": {
      "base_url": "http://localhost:11434"
    }
  }
}</code></pre>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/providers.md" target="_blank" rel="noopener">the full providers reference</a> for endpoint URLs, model lists, and per-provider quirks.</p>
  `;
}
