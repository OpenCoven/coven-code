export const meta = { title: 'Welcome Screen' };

export function render() {
  return `
    <h1>The welcome screen</h1>
    <p class="lead">When you launch <code>coven-code</code> interactively, the home screen opens with a single rounded panel titled <code>Coven Code v&lt;version&gt;</code>. It's the at-a-glance status surface — every value comes from another subsystem, so use it as a jumping-off point rather than a source of truth.</p>

    <h2>Left column</h2>
    <p>Your familiar's portrait (animated glyph for built-ins, static card for daemon-registered familiars) under a <code>Welcome back &lt;user&gt;!</code> greeting. The art is driven by the <code>"familiar"</code> field in your settings; see <a href="#familiars">Familiars</a>.</p>

    <h2>Right column</h2>
    <p>A rotating getting-started tip, then a <strong>Status</strong> block:</p>

    <table>
      <thead>
        <tr><th>Field</th><th>What it shows</th><th>Configured in</th></tr>
      </thead>
      <tbody>
        <tr>
          <td><code>Model</code></td>
          <td>Active model id, or the effective default if unset</td>
          <td><code>model</code> in <a href="#configuration">settings.json</a>, <code>/model</code></td>
        </tr>
        <tr>
          <td><code>Provider</code></td>
          <td>Active provider id (<code>anthropic</code> when unset)</td>
          <td><code>provider</code> in <a href="#configuration">settings.json</a>, see <a href="#providers">Providers</a></td>
        </tr>
        <tr>
          <td><code>Daemon</code></td>
          <td><code>online</code> / <code>offline</code> from a cheap socket check — no RPC</td>
          <td>Install <code>@opencoven/coven</code> to bring it online</td>
        </tr>
        <tr>
          <td><code>Familiar</code></td>
          <td>Current familiar id, with an <code>(F2 to switch)</code> hint</td>
          <td><code>familiar</code> in settings, <code>/familiar</code>, or <kbd>F2</kbd></td>
        </tr>
        <tr>
          <td><code>Goal</code></td>
          <td>Active autonomous goal (only shown when one is set)</td>
          <td><code>/goal &lt;objective&gt;</code></td>
        </tr>
      </tbody>
    </table>

    <p>Press <kbd>F2</kbd> at any time to open the familiar switcher popup.</p>

    <h2>Small-terminal fallback</h2>
    <p>On terminals narrower than ~30 columns or shorter than 11 rows, the panel collapses to a single line — <code>Coven Code v… · &lt;model&gt; · &lt;daemon&gt; · &lt;familiar&gt;</code> — so the essentials stay visible even in a tiny pane.</p>
  `;
}
