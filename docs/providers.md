# LLM Providers

Coven Code supports two providers through a common `LlmProvider` interface: **Anthropic** (Claude) and **Codex** (OpenAI's Codex, accessed through a ChatGPT/Codex OAuth login).

---

## Selecting a Provider

Use the `--provider` flag on any invocation to override the active provider:

```
coven-code --provider anthropic "refactor this module"
coven-code --provider codex "explain this function"
```

The provider can also be set in the engine-global `settings.json`: normally
`~/.coven-code/settings.json` for standalone use or `~/.coven/code/settings.json`
under Coven. `COVEN_CODE_HOME` can override the engine home. For example:

```json
{
  "provider": "anthropic"
}
```

When no provider is specified, Coven Code defaults to **Anthropic**.

---

## Provider Reference

### Anthropic (default)

Claude has two transports. With an Anthropic API key, the engine sends requests
to the Anthropic Messages API. Without an API key, turns run through the local
`claude` binary. That CLI owns its authentication; Coven Code does not import or
replay Claude CLI OAuth tokens.

Set `ANTHROPIC_API_KEY` or configure an engine-owned key. Use `/account connect`
for provider setup and `/account` for account management. The separate
`coven-code auth login` flow requires a configured
`COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID`; it is not a Claude CLI token-import path.

Model availability and defaults depend on the installed catalog and account.
Inspect the engine's resolved models rather than relying on a static price or
context-window table:

```bash
coven-code models anthropic
coven-code models anthropic --json
```

The catalog describes model metadata; it does not guarantee account entitlement.
Verify current provider pricing before estimating costs.

**Configuration:**

```json
{
  "provider": "anthropic",
  "providers": {
    "anthropic": {
      "api_key": "sk-ant-...",
      "models_whitelist": ["claude-sonnet-4-6", "claude-haiku-4-5-20251001"]
    }
  }
}
```

**Base URL override:** Set `ANTHROPIC_BASE_URL` to point at a proxy or local mirror.

---

### Codex

Runs Claude-style coding sessions against OpenAI's Codex using your ChatGPT/Codex subscription. Authentication is OAuth-only — there is no standalone API key path.

**Authentication:** Sign in with `coven-code codex login` (ChatGPT/Codex OAuth). Tokens are stored in the effective engine home; account profiles use `accounts/codex/<id>/`. See [auth.md](auth.md) for the multi-account flow.

Inspect the models available to the installed engine:

```bash
coven-code models codex
coven-code models codex --json
```

Model catalog entries do not guarantee availability for a particular subscription.

**Configuration:**

```json
{
  "provider": "codex"
}
```

Codex credentials are managed entirely through the OAuth login; you do not place a key in `settings.json`.

---

## Per-Provider Configuration in settings.json

The `providers` map in the engine-global `settings.json` accepts per-provider `ProviderConfig` objects:

```json
{
  "provider": "anthropic",
  "providers": {
    "anthropic": {
      "api_key": "sk-ant-...",
      "api_base": "https://api.anthropic.com",
      "enabled": true,
      "models_whitelist": [],
      "models_blacklist": [],
      "options": {}
    }
  }
}
```

**Fields:**

| Field | Type | Description |
|---|---|---|
| `api_key` | string | Override the environment variable API key |
| `api_base` | string | Override the default base URL |
| `enabled` | bool | Enable or disable the provider (default: `true`) |
| `models_whitelist` | array of strings | If non-empty, only listed model IDs are allowed |
| `models_blacklist` | array of strings | Listed model IDs are refused |
| `options` | object | Provider-specific pass-through options |

## Model Whitelist and Blacklist

When `models_whitelist` is non-empty for a provider, only the listed model IDs can be selected for that provider. Any model ID in `models_blacklist` is rejected regardless of the whitelist:

```json
{
  "providers": {
    "anthropic": {
      "models_whitelist": ["claude-opus-4-6", "claude-sonnet-4-6"],
      "models_blacklist": ["claude-haiku-4-5-20251001"]
    }
  }
}
```

The above example allows only `claude-opus-4-6` and `claude-sonnet-4-6` (whitelist minus blacklist).

## Model Registry

Coven Code ships a bundled snapshot of Claude models (`crates/api/assets/models-snapshot.json`). When no model is explicitly set, Coven Code scores available models by priority patterns — flagship family first (`claude-opus-4` > `claude-sonnet-4` > …), then a `latest` suffix, then newest release date — and picks the top match for the active provider. Well-known `claude-*` model prefixes are always routed to Anthropic; `gpt-*` prefixes route to Codex.

On startup the registry refreshes itself from [models.dev](https://models.dev) in the background (cached to disk; the bundled snapshot is the offline fallback). Relevant environment variables:

| Variable | Effect |
|---|---|
| `COVEN_CODE_DISABLE_MODELS_FETCH=1` | Skip the network refresh; use only the bundled snapshot + disk cache |
| `COVEN_CODE_MODELS_URL` / `MODELS_DEV_URL` | Override the models.dev catalog URL |
| `COVEN_CODE_ENABLE_EXPERIMENTAL_MODELS=1` | Include experimental/preview models in the picker |

Inspect the resolved catalog at any time with `coven-code models [<provider>] [--refresh] [--verbose] [--json]`.
