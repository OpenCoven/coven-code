export const meta = { title: 'Slash Commands' };

export function render() {
  return `
    <h1>Slash Commands</h1>
    <p class="lead">Coven Code ships with 70+ slash commands organised into categories — session control, model selection, configuration, code &amp; git workflows, agents, MCP, and more. Type <code>/</code> in the chat input to open the palette, or <kbd>Ctrl+K</kbd> from anywhere.</p>

    <h2>Resolution Order</h2>

    <p>When you type a command, the registry checks in priority order:</p>

    <pre><code data-lang="bash">built-in commands → user command templates → discovered skills → plugin commands</code></pre>

    <p>The first match wins, so user templates can override built-ins of the same name.</p>

    <h2>Browse the Palette</h2>

    <p>Type to filter by name, alias, or description. Click a category chip to narrow the list.</p>

    <div class="demo" x-data="commandExplorer">
      <div class="demo-header">
        <span>slash command explorer · <span x-text="count"></span> / <span x-text="total"></span> shown</span>
      </div>
      <div class="demo-body">
        <div class="explorer-controls">
          <input
            type="text"
            class="explorer-input"
            placeholder="Search commands — try 'commit', 'goal', 'familiar', 'thinking'…"
            x-model="query"
            aria-label="Search slash commands"
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
                <span class="explorer-item-id" x-html="mark(item.id)"></span>
                <span class="explorer-item-cat" x-html="mark(item.category)"></span>
              </div>
              <div class="explorer-item-desc" x-html="mark(item.desc)"></div>
            </div>
          </template>
        </div>
        <div class="explorer-empty" x-show="count === 0">
          No commands match. <a href="#" @click.prevent="clear()" style="color: var(--color-accent);">Clear filters</a>
        </div>
      </div>
    </div>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/commands.md" target="_blank" rel="noopener">the full slash commands reference</a> for every flag, behaviour detail, and the planning/internal commands (<code>/summary</code>, <code>/brief</code>, <code>/thinkback-play</code>, etc.) that aren't surfaced here.</p>
  `;
}
