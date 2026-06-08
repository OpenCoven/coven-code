export const meta = { title: 'Getting Started' };

export function render() {
  return `
    <h1>Getting started</h1>
    <p class="lead">Install, authenticate, and run your first session in under a minute.</p>

    <h2>1. Install</h2>

    <h3>Linux / macOS</h3>
    <pre><code data-lang="bash">curl -fsSL https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh | bash</code></pre>

    <h3>Windows (PowerShell)</h3>
    <pre><code data-lang="bash">irm https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 | iex</code></pre>

    <p>The installer auto-detects your platform/arch, drops <code>coven-code</code> into <code>~/.coven-code/bin/</code>, and adds it to your <code>PATH</code>.</p>

    <h3>npm</h3>
    <pre><code data-lang="bash">npm i -g coven-code</code></pre>

    <h3>From source</h3>
    <pre><code data-lang="bash">git clone https://github.com/OpenCoven/coven-code
cd coven-code/src-rust
cargo install --path crates/cli</code></pre>

    <h2>2. Set your API key</h2>

    <pre><code data-lang="bash">export ANTHROPIC_API_KEY=sk-ant-...</code></pre>

    <p>Or run <code>coven-code /login</code> to authenticate via OAuth (Claude.ai or ChatGPT). Multiple named accounts can coexist; switch with <code>/switch &lt;id&gt;</code>.</p>

    <h2>3. Run interactively</h2>

    <pre><code data-lang="bash">coven-code</code></pre>

    <p>This drops you into the TUI. The first screen is the <a href="#welcome-screen">welcome screen</a>, which surfaces the active model, provider, daemon status, and familiar.</p>

    <p>Or send a single prompt and exit:</p>

    <pre><code data-lang="bash">coven-code --print "explain the auth module"</code></pre>

    <h2>Interactive vs headless</h2>

    <table>
      <thead>
        <tr><th>Mode</th><th>Command</th><th>Use case</th></tr>
      </thead>
      <tbody>
        <tr><td>Interactive TUI</td><td><code>coven-code</code></td><td>Day-to-day coding</td></tr>
        <tr><td>Single prompt</td><td><code>coven-code "task"</code></td><td>Quick one-shot tasks</td></tr>
        <tr><td>Headless print</td><td><code>coven-code --print "task"</code></td><td>Scripts, CI</td></tr>
        <tr><td>JSON output</td><td><code>coven-code --output-format json "task"</code></td><td>Machine consumption</td></tr>
        <tr><td>Stream JSON</td><td><code>coven-code --output-format stream-json "task"</code></td><td>Real-time piping</td></tr>
      </tbody>
    </table>

    <h2>Coven daemon (optional)</h2>

    <p>Coven Code connects natively to the Coven daemon when it's running on your machine. With the daemon active, familiars appear as agents, daemon-registered skills become awareness context, and the welcome panel animates with your familiar's glyph.</p>

    <pre><code data-lang="bash">npm install -g @opencoven/coven</code></pre>

    <p>Coven Code is fully standalone without the daemon — install it separately to unlock the Coven ecosystem features.</p>
  `;
}
