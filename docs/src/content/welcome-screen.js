export const meta = { title: 'Welcome Screen' };

export function render() {
  return `
    <h1>The Welcome Screen</h1>
    <p class="lead">When you launch <code>coven-code</code> interactively, the home screen opens with a single rounded panel titled <code>Coven Code v&lt;version&gt;</code>. It's the at-a-glance status surface — every value comes from another subsystem, so use it as a jumping-off point rather than a source of truth.</p>

    <div class="demo" x-data="welcomeMockup">
      <div class="demo-header">
        <span>tui mock · hover any field</span>
        <div class="demo-header-actions">
          <button class="demo-btn" :aria-pressed="!small" @click="small = false">Full</button>
          <button class="demo-btn" :aria-pressed="small" @click="small = true">Tiny terminal</button>
        </div>
      </div>
      <div class="demo-body">
        <div x-show="!small" class="tui-frame">
          <div class="tui-title">╭─ Coven Code <span class="tui-title-version">v0.0.15</span></div>
          <div class="tui-grid">
            <div>
              <div class="tui-greeting">Welcome back, val!</div>
              <pre class="tui-rustle">      ∧___∧
     ( ・ω・ )
      o_(")(")</pre>
            </div>
            <div class="tui-divider"></div>
            <div>
              <p class="tui-section-title">Tips for getting started</p>
              <p style="color: var(--color-text-secondary); margin-bottom: 12px; font-size: 12px;">Edit AGENTS.md to add instructions for Coven Code</p>
              <p class="tui-section-title">Status</p>
              <div class="tui-status-row" :data-active="field === 'model'" @mouseenter="field = 'model'" @mouseleave="field = null">
                <span class="tui-status-key">Model:</span>
                <span>claude-opus-4-7</span>
              </div>
              <div class="tui-status-row" :data-active="field === 'provider'" @mouseenter="field = 'provider'" @mouseleave="field = null">
                <span class="tui-status-key">Provider:</span>
                <span>anthropic</span>
              </div>
              <div class="tui-status-row" :data-active="field === 'daemon'" @mouseenter="field = 'daemon'" @mouseleave="field = null">
                <span class="tui-status-key">Daemon:</span>
                <span class="tui-status-val-accent">online</span>
              </div>
              <div class="tui-status-row" :data-active="field === 'familiar'" @mouseenter="field = 'familiar'" @mouseleave="field = null">
                <span class="tui-status-key">Familiar:</span>
                <span>raven <span class="tui-hint">(F2 to switch)</span></span>
              </div>
              <div class="tui-status-row" :data-active="field === 'goal'" @mouseenter="field = 'goal'" @mouseleave="field = null">
                <span class="tui-status-key">Goal:</span>
                <span class="tui-status-val-accent">Migrate snake_case API to camelCase</span>
              </div>
            </div>
          </div>
        </div>
        <div x-show="small" class="tui-collapsed">
          <strong>Coven Code</strong> v0.0.15 · claude-opus-4-7 · Daemon: online · Familiar: raven
        </div>
        <div class="tui-explain">
          <template x-if="field">
            <div>
              <p class="tui-explain-title" x-text="explain[field].title"></p>
              <p x-text="explain[field].body"></p>
            </div>
          </template>
          <template x-if="!field">
            <p class="tui-explain-empty">Hover a row above to see what each field is, where it comes from, and how to change it.</p>
          </template>
        </div>
      </div>
    </div>

    <h2>Left Column</h2>
    <p>Your familiar's portrait (animated glyph for built-ins, static card for daemon-registered familiars) under a <code>Welcome back &lt;user&gt;!</code> greeting. The art is driven by the <code>"familiar"</code> field in your settings; see <a href="#familiars">Familiars</a>.</p>

    <h2>Right Column</h2>
    <p>A rotating getting-started tip, then a <strong>Status</strong> block listing the active model, provider, daemon state, familiar (with an <kbd>F2</kbd> to switch hint), and goal (only when set). Hover any row in the mockup above for what each field shows and where to change it.</p>

    <p>Press <kbd>F2</kbd> at any time to open the familiar switcher popup.</p>

    <h2>Small-Terminal Fallback</h2>
    <p>On terminals narrower than ~30 columns or shorter than 11 rows, the panel collapses to a single line — <code>Coven Code v… · &lt;model&gt; · &lt;daemon&gt; · &lt;familiar&gt;</code> — so the essentials stay visible even in a tiny pane.</p>
  `;
}
