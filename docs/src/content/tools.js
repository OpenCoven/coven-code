export const meta = { title: 'Tools' };

export function render() {
  return `
    <h1>Tools Reference</h1>
    <p class="lead">Coven Code ships with 40+ built-in tools across file ops, shell execution, search, web, task management, git, notebooks, and desktop automation. Each tool is gated by a permission level and the active permission mode.</p>

    <h2>Permissions</h2>

    <p>Every tool is gated by a <strong>permission level</strong> (None / ReadOnly / Write / Execute / Dangerous) and the session's active <strong>permission mode</strong>. Modes decide whether each level runs, prompts, or is blocked. Pick a mode below to see how the matrix shifts.</p>

    <p>Permission rules are evaluated per-project and per-user — first match wins. Manage them with <code>/permissions</code>.</p>

    <div class="demo" x-data="permissionViz">
      <div class="demo-header">
        <span>permission visualizer · pick a mode</span>
      </div>
      <div class="demo-body">
        <div class="perm-modes">
          <template x-for="m in modes" :key="m">
            <button class="demo-btn" @click="mode = m" :aria-pressed="mode === m" x-text="m"></button>
          </template>
        </div>
        <p class="perm-mode-blurb" x-text="blurbs[mode]"></p>
        <div class="perm-grid">
          <template x-for="level in levels" :key="level">
            <div class="perm-row" :data-state="cell(level)">
              <div class="perm-level" x-text="level"></div>
              <div class="perm-state">
                <span class="perm-mark" x-text="cellMark(level)"></span>
                <span x-text="cellLabel(level)"></span>
              </div>
              <div class="perm-tools">
                <template x-for="tool in examples[level]" :key="tool">
                  <span class="perm-tool" x-text="tool"></span>
                </template>
              </div>
            </div>
          </template>
        </div>
      </div>
    </div>

    <h2>Browse the Toolkit</h2>

    <p>Click a tool to see its parameters and an example invocation. Write tools enforce read-before-write — a file must have been read in the current session before it can be modified, preventing blind overwrites.</p>

    <div class="demo" x-data="toolsGrid">
      <div class="demo-header">
        <span>tools catalog · click any card</span>
      </div>
      <div class="demo-body">
        <template x-for="cat in categories" :key="cat.name">
          <div class="tools-cat">
            <div class="tools-cat-title" x-text="cat.name"></div>
            <div class="tools-cards">
              <template x-for="tool in cat.tools" :key="tool.name">
                <button
                  type="button"
                  class="tool-card"
                  :aria-expanded="expanded === tool.name"
                  @click="toggle(tool.name)"
                >
                  <div class="tool-card-head">
                    <span class="tool-card-name" x-text="tool.name"></span>
                    <span class="tool-card-level" x-text="tool.level"></span>
                  </div>
                  <div class="tool-card-desc" x-text="tool.desc"></div>
                  <template x-if="expanded === tool.name">
                    <div class="tool-detail" @click.stop>
                      <div class="tool-detail-section" x-show="tool.params.length">
                        <div class="tool-detail-label">Parameters</div>
                        <div class="tool-detail-params">
                          <template x-for="p in tool.params" :key="p">
                            <span class="tool-detail-param" x-text="p"></span>
                          </template>
                        </div>
                      </div>
                      <div class="tool-detail-section">
                        <div class="tool-detail-label">Example</div>
                        <div class="tool-detail-example" x-text="tool.example"></div>
                      </div>
                    </div>
                  </template>
                </button>
              </template>
            </div>
          </div>
        </template>
      </div>
    </div>

    <h2>Other Categories</h2>

    <ul>
      <li><strong>Notebooks</strong> — read and edit Jupyter notebooks</li>
      <li><strong>Desktop automation</strong> — screenshot, click, type (optional <code>computer-use</code> feature)</li>
      <li><strong>MCP tools</strong> — dynamically added when MCP servers connect; see <a href="#mcp">MCP</a></li>
    </ul>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/tools.md" target="_blank" rel="noopener">the full tools reference</a> for parameter schemas, return types, and per-tool quirks across all 40+ built-ins.</p>
  `;
}
