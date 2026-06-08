export const meta = { title: 'Keybindings' };

export function render() {
  return `
    <h1>Keybindings</h1>
    <p class="lead">Coven Code uses a context-aware keybinding system — the same key can have different effects depending on where focus is, and bindings in more specific contexts override broader ones. Customise via the <code>/keybindings</code> editor or <code>~/.coven-code/keybindings.json</code>.</p>

    <h2>Browse shortcuts</h2>

    <p>Type to filter by key, action, or description. Click a context chip to narrow to that scope.</p>

    <div class="demo" x-data="keybindingExplorer">
      <div class="demo-header">
        <span>keybinding explorer · <span x-text="count"></span> / <span x-text="total"></span> shown</span>
      </div>
      <div class="demo-body">
        <div class="explorer-controls">
          <input
            type="text"
            class="explorer-input"
            placeholder="Search keys or actions — try 'ctrl', 'history', 'palette', 'word'…"
            x-model="query"
            aria-label="Search keybindings"
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
          <template x-for="item in filtered" :key="item.id + item.category">
            <div class="explorer-item">
              <div class="explorer-item-head">
                <span class="explorer-item-id key-id" x-text="item.id"></span>
                <span class="explorer-item-cat" x-text="item.category"></span>
              </div>
              <div class="explorer-item-desc" x-text="item.desc"></div>
            </div>
          </template>
        </div>
        <div class="explorer-empty" x-show="count === 0">
          No shortcuts match. <a href="#" @click.prevent="clear()" style="color: var(--color-accent);">Clear filters</a>
        </div>
      </div>
    </div>

    <h2>How contexts work</h2>

    <p>The same key can have different effects depending on which UI surface has focus. Bindings in a more specific context take precedence over a broader one.</p>

    <ul>
      <li><code>global</code> — always active</li>
      <li><code>chat</code> — chat input has focus</li>
      <li><code>confirmation</code> — a permission dialog is open</li>
      <li><code>modelPicker</code> — the model selection overlay is open</li>
      <li><code>commandPalette</code> — the slash command palette is open</li>
      <li><code>search</code> — the inline search bar is open</li>
      <li><code>vim.normal</code> / <code>vim.insert</code> / <code>vim.visual</code> — when vim mode is enabled</li>
    </ul>

    <h2>Customising</h2>

    <p>The interactive editor:</p>

    <pre><code data-lang="bash">/keybindings</code></pre>

    <p>Or edit <code>~/.coven-code/keybindings.json</code> directly:</p>

    <pre><code data-lang="json">{
  "version": 2,
  "bindings": {
    "chat": {
      "openModelPicker": ["Ctrl+Shift+M"]
    },
    "global": {
      "openHelp": ["F1"]
    }
  }
}</code></pre>

    <p>The file uses smart merge so user customisations survive schema upgrades.</p>

    <h2>Vim mode</h2>

    <p>Enable in settings:</p>

    <pre><code data-lang="json">{ "config": { "vim_mode": true } }</code></pre>

    <p>Adds <code>vim.normal</code>, <code>vim.insert</code>, and <code>vim.visual</code> contexts with standard motions, text objects, and a mode indicator in the input gutter.</p>

    <h2>@file injection</h2>

    <p>Type <code>@</code> in the chat input to open the file picker. Typeahead suggests paths from the workspace as you type; press <kbd>Tab</kbd> or <kbd>Enter</kbd> to inject. Size and binary safety checks run before submit.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/keybindings.md" target="_blank" rel="noopener">the full keybindings reference</a> for chord bindings, non-English layout handling, and the complete vim mode binding list.</p>
  `;
}
