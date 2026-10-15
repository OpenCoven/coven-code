# Coven Code Authentication Guide

Coven Code supports Anthropic (Claude) and Codex. Claude can use an Anthropic
API key or the locally authenticated `claude` CLI. Codex uses ChatGPT OAuth.
Coven Code does not import or replay OAuth tokens owned by Claude Code or another
CLI. `coven-code auth import` is removed.

## Claude with an API key

Supply the key through the process environment:

```bash
export ANTHROPIC_API_KEY="<your-key>"
coven-code --provider anthropic
```

In PowerShell:

```powershell
$env:ANTHROPIC_API_KEY = '<your-key>'
coven-code --provider anthropic
```

For CI, inject the key through the CI secret mechanism. Avoid storing actual
keys in committed scripts, command history, or copied diagnostics.

Engine-global settings can also hold a key:

```json
{
  "config": {
    "api_key": "<your-key>"
  }
}
```

The `--api-key` CLI flag supplies a session override, but a literal key on a
command line can appear in shell history or process listings. Prefer secret
injection when available.

Config-based Anthropic key resolution checks the top-level key, then the
Anthropic provider-specific key, then `ANTHROPIC_API_KEY`. Runtime provider
construction also supports the engine-owned credential store. Do not treat the
status command's source label as a universal precedence rule across all runtime
entry points.

## Claude through the local CLI

Without an API key, ordinary Claude turns use the local `claude` executable.
Authenticate in that CLI and make it available on PATH. Its subscription login
stays inside that CLI; copying its tokens into Coven Code is neither required
nor supported.

`coven-code auth status` describes engine-owned credentials. It can report no
engine login even when the local Claude CLI has a usable login. Check that CLI's
own authentication when diagnosing delegated Claude turns.

## Direct Anthropic OAuth

The separate engine-owned OAuth flow requires a client ID registered for Coven
Code. Do not reuse another CLI's client identity:

```bash
export COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID="<registered-client-id>"
coven-code auth login --label work
```

The flow opens an authorization URL and receives a browser callback. Follow the
URL and fallback instructions printed by the flow if the browser cannot open.
The `--console` variant requests the Console flow, which creates an API key:

```bash
coven-code auth login --console --label work
```

Stored bearer credentials are usable only when associated with the configured
Coven Code client identity. An old or imported token file is not proof of a
supported login. The usual Claude setup is an API key or authenticated local
Claude CLI.

## Codex browser login

```bash
coven-code codex login --label personal
coven-code --provider codex
```

Codex uses ChatGPT OAuth, not a standalone OpenAI API key. Follow the browser URL
printed by the engine. The repository's generic RFC 8628 device-code helper is
not wired to a provider and is not an alternative login command.

## Account management

Use the profile ID printed by the login/list command when switching or removing
accounts; a display label does not guarantee a particular generated ID.

| Operation | Anthropic | Codex |
|---|---|---|
| Login | `coven-code auth login [--console] [--label <name>]` | `coven-code codex login [--label <name>]` |
| List profiles | `coven-code auth list` | `coven-code codex list` |
| Switch active profile | `coven-code auth switch <profile-id>` | `coven-code codex switch <profile-id>` |
| Remove selected profile | `coven-code auth remove <profile-id>` | `coven-code codex remove <profile-id>` |
| Logout | `coven-code auth logout` | `coven-code codex logout` |
| Status | `coven-code auth status [--json]` | `coven-code codex status` |

List both providers with `coven-code accounts`; use `coven-code accounts --json`
for registry JSON. Review that output before sharing it: it can include account
identity metadata.

In the TUI, use the canonical account group:

```text
/account connect
/account list
/account login
/account login --codex
/account switch <profile-id>
/account switch --codex <profile-id>
/account logout
/account logout --codex
/account dedupe
```

`/account dedupe` reconciles duplicate profiles; inspect the account list before
using it. The old `/connect`, `/login`, `/logout`, and `/accounts` roots are
retired. See the [generated command reference](slash-commands.md).

## Storage and effective home

The engine home normally is `~/.coven-code` for standalone use and
`~/.coven/code` under Coven. `COVEN_CODE_HOME` overrides it; `COVEN_HOME` can
relocate the Coven home. Inspect overrides and migration symlinks when locating
files. Paths below are relative to the effective engine home:

```text
accounts.json                         # profile registry and active selections
accounts/
  anthropic/<id>/oauth_tokens.json     # engine-owned Anthropic profile
  codex/<id>/codex_tokens.json         # Codex profile
auth.json                             # separate provider credential store
settings.json                         # engine-global configuration
```

`accounts.json` is alongside the `accounts` directory, not inside it. The
credential store is separate from the profile registry. Legacy single-file
credentials may be migrated by the profile loaders; retain a backup before
upgrades and do not manually move live token files to simulate a login.

Keep credential files private and out of version control. Release-specific
private-write guarantees and Windows ACL verification belong to the release
notes; a file path alone does not establish its permissions or durability.

## Status and troubleshooting

```bash
coven-code auth status --json
coven-code codex status
```

`auth status --json` emits a boolean `loggedIn` and fields such as `authMethod`,
`apiProvider`, and `billing`. It exits 0 when engine-owned credentials are
reported available and 1 otherwise. It uses the configured provider; it is not
an account-entitlement test or a live inference check. `codex status` emits a
human-readable active-profile summary and exits 0 when a profile is present,
1 otherwise. It does not implement the same JSON interface.

For missing credentials, confirm the selected provider, effective engine home,
and active profile. For delegated Claude, verify the local CLI's login. For
Codex, use `codex list` and the appropriate `codex switch` or `codex login`.
Expired OAuth credentials may refresh through their owning provider flow;
refresh failures can still require a new login. No universal API-key fallback
is promised for Codex.

Logout affects engine-owned stored state. Anthropic CLI logout also clears the
top-level settings key, but it is not a guarantee that every credential source
has been purged. Environment keys, provider-specific configuration, the separate
credential store, and the local Claude CLI's own login must be considered
separately. Remove or revoke the intended credential at its owning source, then
recheck status; do not delete the shared Coven home to log out.
