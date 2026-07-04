# Coven Code Authentication Guide

Coven Code needs credentials to call the Anthropic API (or sign in to Codex).
This document covers every supported authentication method, multi-account
profile switching, how tokens are stored, and how to check and clear
credentials.

---

## Authentication Methods

Coven Code checks for credentials in the following priority order:

1. `--api-key` flag (highest priority, session-only)
2. `api_key` field in `~/.coven-code/settings.json`
3. `ANTHROPIC_API_KEY` environment variable
4. Tokens for the **active Anthropic profile** under
   `~/.coven-code/accounts/anthropic/<id>/oauth_tokens.json`
5. Legacy `~/.coven-code/oauth_tokens.json` (auto-migrated to a profile on first
   read)

The first non-empty credential found is used.

Codex (OpenAI ChatGPT subscription) accounts follow a parallel system —
multiple profiles stored under `~/.coven-code/accounts/codex/<id>/`, with the
active profile selected via the account registry.

---

## Method 1: API Key

The simplest and most reliable authentication method is a direct API key from
the Anthropic Console.

### Get an API key

1. Log in to [console.anthropic.com](https://console.anthropic.com).
2. Navigate to **Settings > API Keys**.
3. Click **Create Key** and copy the generated `sk-ant-...` key.

### Configure the key

**Option A: Environment variable (recommended)**

Set `ANTHROPIC_API_KEY` in your shell profile. This keeps the key out of any
configuration files that might be committed to version control.

```bash
# Add to ~/.bashrc or ~/.zshrc
export ANTHROPIC_API_KEY="sk-ant-api03-..."
```

On Windows (Command Prompt, permanent):

```cmd
setx ANTHROPIC_API_KEY "sk-ant-api03-..."
```

On Windows (PowerShell profile):

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-api03-..."
# To persist it:
[System.Environment]::SetEnvironmentVariable("ANTHROPIC_API_KEY","sk-ant-api03-...","User")
```

**Option B: Settings file**

Store the key in `~/.coven-code/settings.json`. Ensure the file has restricted
permissions on shared systems.

```json
{
  "config": {
    "api_key": "sk-ant-api03-..."
  }
}
```

**Option C: CLI flag (session-only)**

Pass the key directly for a single run. It is not persisted anywhere.

```bash
coven-code --api-key "sk-ant-api03-..." "your prompt"
```

---

## Method 2: OAuth Login (Browser-based)

Coven Code supports an OAuth 2.0 PKCE flow that authenticates through either
the Anthropic Console or Claude.ai in your browser.

> **Important:** Coven Code must not reuse Claude Code's OAuth client ID.
> Anthropic OAuth requires a client ID registered for Coven Code and supplied
> through `COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID`. Until that first-party client
> is configured, use Method 1 (API key).

### Claude.ai flow

```bash
export COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID=<registered-client-id>
coven-code auth login
```

1. Coven Code generates a PKCE code verifier and code challenge.
2. A temporary localhost HTTP server starts on a random port to receive the
   callback.
3. The authorization URL is printed to the terminal and Coven Code attempts to
   open it in your default browser.
4. Complete the authorization in the browser (Claude.ai login page).
5. The browser redirects to `http://localhost:<port>/callback` with an
   authorization code.
6. Coven Code exchanges the code for tokens via the token endpoint.
7. Tokens are saved under `~/.coven-code/accounts/anthropic/<profile-id>/oauth_tokens.json`
   and the profile is registered as **active** in `~/.coven-code/accounts.json`.

This flow produces a Bearer token (`user:inference` scope) used directly for
API calls.

### Console flow (creates an API key)

```bash
export COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID=<registered-client-id>
coven-code auth login --console
```

This uses the Anthropic Console authorization endpoint. After token exchange,
Coven Code calls the Console API to create a new API key, stores it in the
active profile's `oauth_tokens.json`, and uses it as a standard API key for
subsequent requests (not as a Bearer token).

### Naming the profile

Add `--label <name>` to give the new profile a human-friendly name (otherwise
the id is derived from the JWT email's local-part). This becomes the id you
use when running `coven-code auth switch`:

```bash
coven-code auth login --label work
coven-code auth login --label personal
coven-code auth switch personal
```

### Manual fallback

If the browser does not open automatically, Coven Code prints the full
authorization URL. Copy and paste it into a browser. After you authorize,
paste the authorization code shown in the browser back into the terminal
when prompted.

---

## Multi-Account Profiles

Coven Code stores **multiple named accounts per provider** and lets you switch
between them without re-logging-in. Supported providers today: **Anthropic**
(Claude.ai / Console) and **Codex** (OpenAI ChatGPT subscription).

This is useful for separating work and personal accounts, juggling
Pro/Max/Team plans, or testing against multiple organizations.

### On-disk layout

```
~/.coven-code/
├── accounts.json                              # registry (active + metadata)
└── accounts/
    ├── anthropic/
    │   ├── work/oauth_tokens.json
    │   └── personal/oauth_tokens.json
    └── codex/
        └── work/codex_tokens.json
```

`accounts.json` schema (excerpt):

```json
{
  "version": 1,
  "providers": {
    "anthropic": {
      "active": "personal",
      "profiles": {
        "work":     { "id": "work",     "email": "kuber@company.example",  "subscription_tier": "max", "added_at": "2026-05-25T19:00:00Z" },
        "personal": { "id": "personal", "email": "kuber@personal.example", "subscription_tier": "pro", "added_at": "2026-05-25T19:05:00Z" }
      }
    },
    "codex": {
      "active": "work",
      "profiles": { "work": { "id": "work", "email": "kuber@company.example" } }
    }
  }
}
```

### CLI

`coven-code auth` and `coven-code codex` are symmetric — same subcommands for both
providers:

```bash
# Add accounts (each login becomes its own profile)
coven-code auth login                       # Claude.ai, requires COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID
coven-code auth login --console             # Console / API-key flow, requires COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID
coven-code auth login --label work          # name the profile
coven-code codex login                      # ChatGPT/Codex OAuth
coven-code codex login --label personal

# Inspect
coven-code auth status                      # show active Anthropic profile
coven-code auth list                        # all Anthropic profiles
coven-code codex list                       # all Codex profiles
coven-code accounts                         # both at once (use --json for JSON)

# Switch the active account
coven-code auth switch work
coven-code codex switch personal

# Remove a stored profile
coven-code auth remove work                 # delete profile + tokens dir
coven-code codex remove personal

# Logout (clears tokens for the active profile)
coven-code auth logout
coven-code codex logout
```

`coven-code auth status` and `coven-code codex status` exit `0` when logged in and
`1` otherwise, so they can drive scripts:

```bash
if coven-code codex status > /dev/null; then
  echo "Codex login present"
fi
```

### Slash commands

Inside the interactive REPL the same operations are available as slash
commands — Anthropic is the default, pass `--codex` to target Codex:

```
/login                          # OAuth login (Claude.ai)
/login --console                # API-key flow
/login --codex                  # add a Codex account
/login --label work             # name the new profile
/logout                         # clear active Anthropic credentials
/logout --codex                 # clear active Codex credentials
/logout --all                   # purge every stored Anthropic profile
/accounts                       # list every stored account
/login switch personal          # set active Anthropic to "personal"
/login switch --codex work      # set active Codex to "work"
```

`/accounts` lists every profile with a `*` next to the active one and shows
email and subscription tier when known.

### Identity detection

When you log in, Coven Code decodes the JWT id_token (or access token for Codex)
to extract your email and provider-side account_id. If a stored profile
already matches that identity, the existing profile is refreshed instead of
a duplicate being created — re-logging-in the same account is idempotent.

### Backward compatibility

If you previously used Coven Code (with the older single-file storage), your
existing tokens are auto-migrated on first read:

- `~/.coven-code/oauth_tokens.json` → `~/.coven-code/accounts/anthropic/<derived>/oauth_tokens.json`
- `~/.coven-code/codex_tokens.json` → `~/.coven-code/accounts/codex/<derived>/codex_tokens.json`

The legacy files are removed after a successful migration. No manual action
needed.

---

## Method 3: Headless / CI

For headless or server environments where opening a browser is not practical,
the API key method (Method 1) is the recommended approach. Set
`ANTHROPIC_API_KEY` in the environment before running Coven Code in a CI/CD or
server context.

```bash
# Headless / CI example
ANTHROPIC_API_KEY="sk-ant-..." coven-code --print "summarize the last 10 commits"
```

---

## Token Storage

### Anthropic OAuth tokens (per profile)

Each Anthropic account profile has its own file:

```
~/.coven-code/accounts/anthropic/<profile-id>/oauth_tokens.json
```

The file contains the access token, optional refresh token, expiry timestamp,
granted scopes, and account email. Example structure:

```json
{
  "access_token": "...",
  "refresh_token": "...",
  "expires_at_ms": 1700000000000,
  "scopes": ["user:inference", "user:profile"],
  "email": "you@example.com",
  "api_key": "sk-ant-..."
}
```

The active profile pointer lives in `~/.coven-code/accounts.json` (see
[Multi-Account Profiles](#multi-account-profiles)). Files are written with
user-only permissions (`600` on Unix). Do not commit them to version control.

### Codex tokens (per profile)

```
~/.coven-code/accounts/codex/<profile-id>/codex_tokens.json
```

Contains the OpenAI access token, refresh token, account_id, and expiry.

### Provider credential store

API keys stored without going through an OAuth profile live in:

```
~/.coven-code/auth.json
```

This file is keyed by provider ID and contains either an `api` credential
(plain key) or an `oauth` credential (access + refresh token pair):

```json
{
  "credentials": {
    "anthropic": { "type": "api", "key": "sk-ant-..." }
  }
}
```

> **Note:** `~/.coven-code/auth.json` is the credential cache for simple
> API-key storage. It is **distinct** from `~/.coven-code/accounts.json`,
> which is the multi-account registry for Anthropic/Codex OAuth profiles.

---

## Checking Authentication Status

```bash
coven-code auth status
```

Prints a human-readable summary:

```
Logged in.
  API provider: Anthropic
  Login method: API Key
  Billing mode: API
  Key source:   ANTHROPIC_API_KEY
```

For machine-readable output:

```bash
coven-code auth status --json
```

Example JSON output:

```json
{
  "loggedIn": true,
  "authMethod": "api_key",
  "apiProvider": "Anthropic",
  "billing": "API",
  "apiKeySource": "ANTHROPIC_API_KEY"
}
```

The exit code is `0` when logged in, `1` when not logged in. This makes
`auth status` suitable for scripting:

```bash
if coven-code auth status > /dev/null 2>&1; then
  echo "credentials present"
fi
```

---

## Logging Out

By default, `logout` removes the **active** account's tokens and drops that
profile from the registry; other stored profiles are untouched, so a stored
secondary profile becomes the candidate for next selection.

```bash
# Remove the active Anthropic profile
coven-code auth logout

# Remove the active Codex profile
coven-code codex logout

# Or from inside the REPL
/logout
/logout --codex
```

To purge every stored profile for a provider (and clear any API key in
`settings.json`):

```
/logout --all          # Anthropic
/logout --codex --all  # Codex
```

API keys set via environment variables are not affected by `logout`; remove
them from your shell profile manually.

To delete a specific stored profile without making it active first:

```bash
coven-code auth remove work
coven-code codex remove personal
```

---

## Token Refresh

When Coven Code loads OAuth tokens for the active profile and the access token
is expired, it automatically attempts a silent refresh:

1. A `POST` request is sent to the provider's token endpoint with the stored
   refresh token.
2. If successful, the new access token (and optionally a new refresh token)
   is written back to the same per-profile token file.
3. The refreshed token is used for the current session.

If the refresh fails (network error, expired refresh token, revoked grant),
Coven Code falls back to any configured API key. If no API key is available,
authentication fails and you must run `coven-code auth login` (optionally with
`--label <name>` to reuse a profile id) again.

For first-run setup in the TUI, `/connect` is usually the fastest path: it can
collect a Claude API key, import a local Claude Code/ant login, start Claude.ai
OAuth when `COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID` is configured, or start Codex
browser login.

---

## Providers

Coven Code supports two providers: **Anthropic** (Claude) and **Codex**.
The active provider looks for credentials in this order:

1. `api_key` in the provider's entry under `providers` in `settings.json`
2. The provider-specific environment variable (see table below)
3. The credential stored in `~/.coven-code/auth.json`
4. The active OAuth profile under `~/.coven-code/accounts/<provider>/`

### Provider environment variables

| Provider | Environment variable |
|----------|---------------------|
| `anthropic` | `ANTHROPIC_API_KEY` |
| `codex` | OAuth only (`coven-code codex login`) |

### Switch providers at runtime

```bash
# Use Anthropic for this session
coven-code --provider anthropic "your prompt"

# Use Codex (requires a Codex OAuth login)
coven-code --provider codex "your prompt"

# Or via environment variable
COVEN_CODE_PROVIDER=codex coven-code "your prompt"
```

---

## Security Recommendations

- Store API keys in environment variables or a secrets manager rather than in
  `settings.json`, especially on shared or CI systems.
- Restrict permissions on `~/.coven-code/` to your user only:
  ```bash
  chmod 700 ~/.coven-code
  chmod 700 ~/.coven-code/accounts
  chmod 600 ~/.coven-code/accounts.json
  chmod 600 ~/.coven-code/auth.json
  chmod 600 ~/.coven-code/settings.json
  find ~/.coven-code/accounts -type f -name '*tokens.json' -exec chmod 600 {} +
  ```
  Coven Code already sets `0600` on `accounts.json` automatically on Unix; the
  command above is the belt-and-braces version that also covers the per-
  profile token files.
- Do not commit `~/.coven-code/` to version control.
- Add `.coven-code/` to your project's `.gitignore` to prevent accidentally
  committing project-level settings files that may contain keys.
- Rotate API keys periodically from the Anthropic Console.
- Use `coven-code auth logout` on shared machines before logging out of your
  user session.
