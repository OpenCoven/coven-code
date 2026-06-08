export const meta = { title: 'Keybindings' };

export function render() {
  return `
    <h1>Keybindings</h1>
    <p class="lead">Coven Code uses a context-aware keybinding system — the same key can have different effects depending on where focus is, and bindings in more specific contexts override broader ones. Customise via the <code>/keybindings</code> editor or <code>~/.coven-code/keybindings.json</code>.</p>

    <h2>Global context</h2>

    <p>Active everywhere.</p>

    <table class="shortcut-table">
      <thead><tr><th>Key</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><kbd>Ctrl+C</kbd></td><td>Interrupt the current operation (non-rebindable)</td></tr>
        <tr><td><kbd>Ctrl+D</kbd></td><td>Exit Coven Code (non-rebindable)</td></tr>
        <tr><td><kbd>Ctrl+L</kbd></td><td>Redraw the terminal screen</td></tr>
        <tr><td><kbd>Ctrl+R</kbd></td><td>Open interactive history search</td></tr>
        <tr><td><kbd>Ctrl+B</kbd></td><td>Create a new git branch</td></tr>
        <tr><td><kbd>Alt+H</kbd></td><td>Open the help panel</td></tr>
        <tr><td><kbd>F2</kbd></td><td>Open familiar switcher</td></tr>
      </tbody>
    </table>

    <h2>Chat context</h2>

    <p>Active when focus is in the chat input.</p>

    <table class="shortcut-table">
      <thead><tr><th>Key</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><kbd>Enter</kbd></td><td>Submit message</td></tr>
        <tr><td><kbd>Shift+Enter</kbd> / <kbd>Ctrl+J</kbd></td><td>Insert a literal newline</td></tr>
        <tr><td><kbd>Up</kbd> / <kbd>Ctrl+O</kbd></td><td>Previous in input history</td></tr>
        <tr><td><kbd>Down</kbd> / <kbd>Ctrl+I</kbd></td><td>Next in input history</td></tr>
        <tr><td><kbd>Tab</kbd></td><td>Indent (or cycle completions)</td></tr>
        <tr><td><kbd>Page Up</kbd> / <kbd>Page Down</kbd></td><td>Scroll conversation</td></tr>
        <tr><td><kbd>Ctrl+A</kbd> / <kbd>Ctrl+E</kbd></td><td>Move cursor to line start / end</td></tr>
        <tr><td><kbd>Ctrl+Left</kbd> / <kbd>Ctrl+Right</kbd></td><td>Move one word</td></tr>
        <tr><td><kbd>Alt+Left</kbd> / <kbd>Alt+Right</kbd></td><td>Jump to previous/next message</td></tr>
        <tr><td><kbd>Ctrl+Shift+A</kbd></td><td>Open model picker</td></tr>
        <tr><td><kbd>Ctrl+K</kbd></td><td>Open slash command palette</td></tr>
        <tr><td><kbd>Ctrl+U</kbd></td><td>Kill to start of line</td></tr>
        <tr><td><kbd>Ctrl+W</kbd> / <kbd>Alt+Backspace</kbd></td><td>Delete word before cursor</td></tr>
        <tr><td><kbd>Alt+D</kbd></td><td>Delete word after cursor</td></tr>
        <tr><td><kbd>Ctrl+F</kbd></td><td>Find within current conversation</td></tr>
        <tr><td><kbd>Ctrl+Shift+F</kbd></td><td>Global codebase search</td></tr>
        <tr><td><kbd>F3</kbd> / <kbd>Shift+F3</kbd></td><td>Next / previous search match</td></tr>
      </tbody>
    </table>

    <h2>Confirmation context</h2>

    <p>Active during yes/no permission prompts.</p>

    <table class="shortcut-table">
      <thead><tr><th>Key</th><th>Action</th></tr></thead>
      <tbody>
        <tr><td><kbd>Y</kbd></td><td>Approve</td></tr>
        <tr><td><kbd>N</kbd></td><td>Deny</td></tr>
        <tr><td><kbd>A</kbd></td><td>Approve + add permanent allow rule</td></tr>
        <tr><td><kbd>Enter</kbd></td><td>Accept the highlighted default</td></tr>
        <tr><td><kbd>Escape</kbd></td><td>Cancel (deny)</td></tr>
      </tbody>
    </table>

    <h2>Contexts</h2>

    <table>
      <thead><tr><th>Context</th><th>Active when</th></tr></thead>
      <tbody>
        <tr><td><code>global</code></td><td>Always</td></tr>
        <tr><td><code>chat</code></td><td>Chat input has focus</td></tr>
        <tr><td><code>confirmation</code></td><td>Permission dialog is open</td></tr>
        <tr><td><code>modelPicker</code></td><td>Model selection overlay open</td></tr>
        <tr><td><code>commandPalette</code></td><td>Slash command palette open</td></tr>
        <tr><td><code>search</code></td><td>Inline search bar open</td></tr>
        <tr><td><code>vim.normal</code> / <code>vim.insert</code> / <code>vim.visual</code></td><td>Vim mode active in the matching mode</td></tr>
      </tbody>
    </table>

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
