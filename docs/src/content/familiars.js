export const meta = { title: 'Familiars' };

export function render() {
  return `
    <h1>Coven Familiars</h1>
    <p class="lead">Coven Code integrates natively with the Coven daemon's familiar roster. When the daemon is installed and running, every familiar you have configured under <code>~/.coven/</code> is automatically available inside Coven Code as a selectable agent persona — no extra setup required.</p>

    <h2>What Is a Familiar?</h2>

    <p>A familiar is a named AI persona defined in the Coven ecosystem. Each familiar has an identity (display name, emoji, pronouns), a role description, and optional metadata used to shape how the model presents itself and reasons about tasks. Familiars are user-defined and live in <code>~/.coven/familiars.toml</code>, managed by the Coven daemon.</p>

    <p>For example, a minimal Coven setup might have:</p>

    <table>
      <thead><tr><th>ID</th><th>Name</th><th>Role</th></tr></thead>
      <tbody>
        <tr><td><code>dev</code></td><td>Dev 🤖</td><td>Code-first implementation agent</td></tr>
        <tr><td><code>research</code></td><td>Research 🧙</td><td>Research and reasoning</td></tr>
        <tr><td><code>writer</code></td><td>Writer ✍️</td><td>Writing and communication</td></tr>
      </tbody>
    </table>

    <p>You define your own familiars — the names, roles, and roster are entirely yours.</p>

    <h2>How Familiars Appear</h2>

    <p>When the daemon is present, <code>load_agent_definitions()</code> reads <code>~/.coven/familiars.toml</code> and converts each familiar into an <code>AgentDefinition</code> with:</p>

    <ul>
      <li><strong>source:</strong> <code>coven:familiar:&lt;id&gt;</code> — distinguishes them from user-defined agents</li>
      <li><strong>instructions:</strong> a synthesised system-prompt body that captures the familiar's name, role, and description</li>
      <li><strong>memory_scope:</strong> <code>workspace</code> — familiars have full workspace context by default</li>
      <li><strong>model:</strong> inherits the session default (no override unless the user sets one)</li>
    </ul>

    <p>Familiars are appended <strong>after</strong> workspace agents in the list. If a user-defined agent shares the same display name as a familiar, the user definition wins.</p>

    <h2>Where Familiars Show Up</h2>

    <ol>
      <li>The <strong>welcome panel</strong> (top-left of the home screen): glyph, name, access tier dot, and on wider terminals the role and an accent rule. See <a href="#welcome-screen">Welcome Screen</a>.</li>
      <li>The <strong>F2 switcher popup</strong>: one row per saved familiar, each painted in that familiar's accent palette with a coloured tier dot.</li>
      <li>The <strong><code>/familiar</code> detail view</strong>: the card appears above the persona preview when you select a familiar-sourced agent.</li>
    </ol>

    <h2>Switching Familiars</h2>

    <p>From the TUI, press <kbd>F2</kbd> to open the switcher when a saved familiar roster exists, or use the slash command:</p>

    <pre><code data-lang="bash">/familiar raven
/familiar list</code></pre>

    <p>From the CLI:</p>

    <pre><code data-lang="bash">coven-code agents list
coven-code agents use raven</code></pre>

    <p>Or set it persistently in <code>~/.coven-code/settings.json</code>:</p>

    <pre><code data-lang="json">{
  "familiar": "raven"
}</code></pre>

    <h2>Without the Daemon</h2>

    <p>Coven Code works fully standalone. Without the daemon, familiars degrade gracefully:</p>

    <ul>
      <li>The welcome panel shows <code>Familiar: none</code> until a saved roster familiar is selected.</li>
      <li>The <code>/familiar</code> overlay shows only workspace agents.</li>
      <li><kbd>F2</kbd> opens a switcher only when a saved familiar roster exists.</li>
      <li><code>/familiar</code> lists and selects only familiars from <code>~/.coven/familiars.toml</code>.</li>
    </ul>

    <p>After <code>/familiar reset-roster</code> or <code>coven-code agents reset</code>, Coven Code removes <code>~/.coven/familiars.toml</code>, renders <code>Familiar: none</code>, hides the footer familiar label, and does not open the F2 switcher until the roster file is recreated.</p>

    <p>Install the daemon to unlock the full roster:</p>

    <pre><code data-lang="bash">npm install -g @opencoven/coven</code></pre>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/familiars.md" target="_blank" rel="noopener">the full familiars reference</a> for access tiers, persona authoring, and CLI integration.</p>
  `;
}
