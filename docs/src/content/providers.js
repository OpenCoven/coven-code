export const meta = { title: 'Providers' };

export function render() {
  return `
    <h1>LLM providers</h1>
    <p class="lead">Coven Code supports a wide range of LLM providers through a unified <code>LlmProvider</code> trait. Switching between them requires only a configuration change.</p>

    <h2>Selecting a provider</h2>

    <p>Use <code>--provider</code> on any invocation to override the active provider:</p>

    <pre><code data-lang="bash">coven-code --provider openai "refactor this module"
coven-code --provider ollama "explain this function"
coven-code --provider groq --model llama-3.3-70b-versatile "write tests"</code></pre>

    <p>Or set it persistently in <code>~/.coven-code/settings.json</code>:</p>

    <pre><code data-lang="json">{
  "provider": "openai"
}</code></pre>

    <p>When no provider is specified, Coven Code defaults to <strong>Anthropic</strong>.</p>

    <h2>Supported providers</h2>

    <table>
      <thead><tr><th>Provider</th><th>ID</th><th>Auth</th></tr></thead>
      <tbody>
        <tr><td>Anthropic Claude</td><td><code>anthropic</code></td><td><code>ANTHROPIC_API_KEY</code> or OAuth</td></tr>
        <tr><td>OpenAI</td><td><code>openai</code></td><td><code>OPENAI_API_KEY</code></td></tr>
        <tr><td>Google Gemini</td><td><code>google</code></td><td><code>GOOGLE_API_KEY</code></td></tr>
        <tr><td>AWS Bedrock</td><td><code>bedrock</code></td><td>AWS credentials chain</td></tr>
        <tr><td>Azure OpenAI</td><td><code>azure</code></td><td><code>AZURE_OPENAI_API_KEY</code> + endpoint</td></tr>
        <tr><td>Ollama (local)</td><td><code>ollama</code></td><td>none (local socket)</td></tr>
        <tr><td>Groq</td><td><code>groq</code></td><td><code>GROQ_API_KEY</code></td></tr>
        <tr><td>Mistral</td><td><code>mistral</code></td><td><code>MISTRAL_API_KEY</code></td></tr>
        <tr><td>DeepSeek</td><td><code>deepseek</code></td><td><code>DEEPSEEK_API_KEY</code></td></tr>
        <tr><td>xAI</td><td><code>xai</code></td><td><code>XAI_API_KEY</code></td></tr>
        <tr><td>Cohere</td><td><code>cohere</code></td><td><code>COHERE_API_KEY</code></td></tr>
        <tr><td>OpenRouter</td><td><code>openrouter</code></td><td><code>OPENROUTER_API_KEY</code></td></tr>
        <tr><td>Together AI</td><td><code>together</code></td><td><code>TOGETHER_API_KEY</code></td></tr>
        <tr><td>Perplexity</td><td><code>perplexity</code></td><td><code>PERPLEXITY_API_KEY</code></td></tr>
        <tr><td>GitHub Copilot</td><td><code>copilot</code></td><td>Copilot OAuth</td></tr>
        <tr><td>Cerebras</td><td><code>cerebras</code></td><td><code>CEREBRAS_API_KEY</code></td></tr>
        <tr><td>LM Studio</td><td><code>lmstudio</code></td><td>none (local HTTP)</td></tr>
        <tr><td>LLaMA.cpp</td><td><code>llamacpp</code></td><td>none (local HTTP)</td></tr>
      </tbody>
    </table>

    <h2>Anthropic (default)</h2>

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

    <h2>Per-provider configuration</h2>

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
