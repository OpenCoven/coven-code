# Project plugin trust

Plugins discovered in `<project>/.coven-code/plugins/` start disabled. Use
`/plugin list` and `/plugin info <name>` to inspect them; discovery, listing,
and reloading do not grant permission to execute a project plugin.

Run `/plugin enable <name>` to authorize a plugin. Consent is stored in the
engine-global user `settings.json`, under `config_home()` (normally
`~/.coven-code`, or `~/.coven/code` under Coven). Repository settings cannot
authorize plugins.

A project grant identifies the canonical project folder and canonical plugin
folder, including the project source. Symlink aliases of those same folders
share the grant. A different project or plugin folder requires its own grant,
even when the plugin name is identical. Legacy name-only enable entries do not
authorize project plugins.

This is folder trust: consent covers future changes within the trusted folder,
including updates to the manifest, commands, and hooks. It does not pin a
content hash. Manifest capabilities are separate from trust; listing a
capability does not authorize a project plugin.

Missing or empty manifest `capabilities` deny every action requiring a grant,
including for user-global plugins. Declare only the actions a plugin needs:

| Contribution | Required capabilities |
| --- | --- |
| Static command response | None |
| Markdown command prompt | `read_files` |
| Shell command or lifecycle hook | `shell` |
| MCP stdio server | `mcp` and `shell` |
| MCP HTTP or SSE server | `mcp` and `network` |
| LSP executable configuration | `shell` |

Unknown MCP transports are rejected. LSP declarations are filtered but there
is no active LSP launcher. These checks gate engine entry points; they are not
an OS sandbox. An allowed shell command retains the process's filesystem and
network access.

Project hook execution checks the caller's canonical working directory. These
hooks run only for their owning project, including through canonical symlink
aliases. Missing or unresolvable project identities deny execution. This also
applies to concurrent/background callers while another project's hooks are
cached globally. User-global hooks remain available across projects.

`/plugin disable <name>` saves the disabled state and removes the plugin's
commands and registered hooks from the current session. Reloads apply saved
preferences. Resuming a session in another project refreshes active plugin
commands and hooks against that project's saved grants.

Existing MCP connections and contributed server configurations remain in the
session; reconnecting can reuse them after a plugin is disabled or its grants
change. Restart the session to stop using those connections and configurations.
Plugin-owned MCP configuration withdrawal and runtime retirement remain a
required follow-up; capability filtering does not close the full plugin supply
chain findings (SEC-SUPPLY-1/2).

Plugins explicitly installed in the user-global plugin directory retain their
trusted default. Their name-based enable and disable settings continue to apply.
User-global plugins take precedence over project plugins with the same name.

If settings cannot be read, parsed, or saved, enable/disable returns an error
instead of reporting that consent was saved. Loading with unreadable or invalid
user settings leaves plugins disabled until the settings are repaired.
