export const meta = { title: 'Getting Started' };

export function render() {
  return `
    <h1>Getting Started</h1>
    <p class="lead">Install, authenticate, and run your first session in under a minute.</p>

    <h2>1. Install</h2>

    <h3>Linux / macOS</h3>
    <pre><code data-lang="bash">curl -fsSL https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh | bash</code></pre>

    <h3>Windows (PowerShell)</h3>
    <pre><code data-lang="bash">irm https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 | iex</code></pre>

    <p>The installer auto-detects your platform/arch, drops <code>coven-code</code> into <code>~/.coven-code/bin/</code>, and adds it to your <code>PATH</code>.</p>

    <h3>npm</h3>
    <pre><code data-lang="bash">npm i -g coven-code</code></pre>

    <h3>From Source</h3>
    <pre><code data-lang="bash">git clone https://github.com/OpenCoven/coven-code
cd coven-code/src-rust
cargo install --path crates/cli</code></pre>

    <h2>2. Set Your API Key</h2>

    <pre><code data-lang="bash">export ANTHROPIC_API_KEY=sk-ant-...</code></pre>

    <p>Or run <code>coven-code /login</code> to authenticate via OAuth (Claude.ai or ChatGPT). Multiple named accounts can coexist; switch with <code>/switch &lt;id&gt;</code>.</p>

    <h2>3. Run Interactively</h2>

    <pre><code data-lang="bash">coven-code</code></pre>

    <p>This drops you into the TUI. The first screen is the <a href="#welcome-screen">welcome screen</a>, which surfaces the active model, provider, daemon status, and familiar.</p>

    <p>Or send a single prompt and exit:</p>

    <pre><code data-lang="bash">coven-code --print "explain the auth module"</code></pre>

    <h2>Interactive vs Headless</h2>

    <div class="demo">
      <div class="demo-header">
        <span>five run modes · pick the one that fits the situation</span>
      </div>
      <div class="demo-body">
        <div class="compare compare-5">
          <div class="compare-card">
            <div class="compare-card-name">interactive</div>
            <span class="compare-card-tag">day-to-day</span>
            <div class="compare-card-desc">Full ratatui TUI with streaming, slash commands, permission dialogs, session history. The default when you launch with no args.</div>
            <div class="compare-card-cmd">coven-code</div>
          </div>

          <div class="compare-card">
            <div class="compare-card-name">single prompt</div>
            <span class="compare-card-tag">one-shot</span>
            <div class="compare-card-desc">One pass over a single task, then exit. TUI still renders the run so you can watch tools fire in real time.</div>
            <div class="compare-card-cmd">coven-code "task"</div>
          </div>

          <div class="compare-card">
            <div class="compare-card-name">headless print</div>
            <span class="compare-card-tag">scripts · CI</span>
            <div class="compare-card-desc">Plain text output to stdout — no TUI, no colour codes, no permission prompts. Use in shell pipelines and CI runners.</div>
            <div class="compare-card-cmd">coven-code --print "task"</div>
          </div>

          <div class="compare-card">
            <div class="compare-card-name">json output</div>
            <span class="compare-card-tag">machine</span>
            <div class="compare-card-desc">Single JSON document with the full run transcript, tool calls, and final result. Parse with jq or feed into downstream tooling.</div>
            <div class="compare-card-cmd">coven-code --output-format json "task"</div>
          </div>

          <div class="compare-card">
            <div class="compare-card-name">stream-json</div>
            <span class="compare-card-tag">real-time</span>
            <div class="compare-card-desc">Newline-delimited JSON events as they happen — useful for streaming progress into another process or live UI.</div>
            <div class="compare-card-cmd">coven-code --output-format stream-json "task"</div>
          </div>
        </div>
      </div>
    </div>

    <h2>Coven Daemon (Optional)</h2>

    <p>Coven Code connects natively to the Coven daemon when it's running on your machine. With the daemon active, familiars appear as agents, daemon-registered skills become awareness context, and the welcome panel animates with your familiar's glyph.</p>

    <pre><code data-lang="bash">npm install -g @opencoven/coven</code></pre>

    <p>Coven Code is fully standalone without the daemon — install it separately to unlock the Coven ecosystem features.</p>
  `;
}
