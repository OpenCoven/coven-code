export const meta = { title: 'Getting Started' };

export function render() {
  return `
    <h1>Getting Started</h1>
    <p class="lead">Install, authenticate, and run your first session in under a minute.</p>

    <h2>1. Install</h2>

    <h3>npm</h3>
    <pre><code data-lang="bash">npm install -g @opencoven/coven</code></pre>

    <p>This installs the <code>coven</code> CLI. Run <code>coven</code> with no arguments, or <code>coven tui</code> explicitly, for the interactive UI.</p>

    <h3>From Source</h3>
    <pre><code data-lang="bash">git clone https://github.com/OpenCoven/coven-code
cd coven-code/src-rust
cargo install --path crates/cli</code></pre>

    <h2>2. Set Your API Key</h2>

    <pre><code data-lang="bash">export ANTHROPIC_API_KEY=sk-ant-...</code></pre>

    <p>Or launch <code>coven</code> and run <code>/login</code> to authenticate via OAuth (Claude.ai or ChatGPT). Multiple named accounts can coexist; switch with <code>/switch &lt;id&gt;</code>.</p>

    <h2>3. Run Interactively</h2>

    <pre><code data-lang="bash">coven</code></pre>

    <p>This drops you into the TUI. The first screen is the <a href="#welcome-screen">welcome screen</a>, which surfaces the active model, provider, daemon status, and familiar.</p>

    <p>Or launch a direct harness session:</p>

    <pre><code data-lang="bash">coven run codex "explain the auth module"</code></pre>

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
            <div class="compare-card-cmd">coven</div>
          </div>

          <div class="compare-card">
            <div class="compare-card-name">direct harness</div>
            <span class="compare-card-tag">one-shot</span>
            <div class="compare-card-desc">Run one named harness against a task from the Coven CLI.</div>
            <div class="compare-card-cmd">coven run codex "task"</div>
          </div>

          <div class="compare-card">
            <div class="compare-card-name">claude harness</div>
            <span class="compare-card-tag">alternate</span>
            <div class="compare-card-desc">Use Claude Code through the same Coven harness runner.</div>
            <div class="compare-card-cmd">coven run claude "task"</div>
          </div>

          <div class="compare-card">
            <div class="compare-card-name">sessions json</div>
            <span class="compare-card-tag">machine</span>
            <div class="compare-card-desc">List known Coven sessions as JSON for scripts, dashboards, or local workflow tooling.</div>
            <div class="compare-card-cmd">coven sessions --json</div>
          </div>

          <div class="compare-card">
            <div class="compare-card-name">stream-json</div>
            <span class="compare-card-tag">real-time</span>
            <div class="compare-card-desc">Newline-delimited JSON events as they happen — useful for streaming progress into another process or live UI.</div>
            <div class="compare-card-cmd">coven run codex "task" --stream-json</div>
          </div>
        </div>
      </div>
    </div>

    <h2>Coven Daemon (Optional)</h2>

    <p>Coven Code connects natively to the Coven daemon when it's running on your machine. With the daemon active, familiars appear as agents, daemon-registered skills become awareness context, and the welcome panel animates with your familiar's glyph.</p>

    <pre><code data-lang="bash">npm install -g @opencoven/coven
coven daemon start</code></pre>

    <p>Coven Code is fully standalone without the daemon — install it separately to unlock the Coven ecosystem features.</p>
  `;
}
