# LLM Providers

Coven Code talks to Claude through a unified provider abstraction. Every provider implements the same `LlmProvider` trait, so switching between authentication modes requires only a configuration change. Two providers are supported: **Anthropic** (Claude) and **Codex** (OpenAI's Codex, accessed through a ChatGPT/Codex OAuth login).

---

## Selecting a Provider

Use the `--provider` flag on any invocation to override the active provider:

```
coven-code --provider anthropic "refactor this module"
coven-code --provider codex "explain this function"
```

The provider can also be set persistently in `~/.coven-code/settings.json`:

```json
{
  "provider": "anthropic"
}
```

When no provider is specified, Coven Code defaults to **Anthropic**.

---

## Provider Reference

### Anthropic (default)

The default provider. Uses the `/v1/messages` streaming endpoint.

**Authentication:** `ANTHROPIC_API_KEY` environment variable, set `api_key` in `settings.json`, or sign in with an Anthropic OAuth profile (Claude.ai / Console). See [auth.md](auth.md) for the OAuth flow.

**Default model:** when no model is set, Coven Code picks the highest-priority
available Claude for the active provider — the newest flagship Opus. With the
live [models.dev](https://models.dev) catalog (fetched and cached on startup)
that is `claude-opus-4-8`; offline, the bundled snapshot tops out at
`claude-opus-4-7`. See [Model Registry](#model-registry) for the selection rules.

**Selected models (offline bundled snapshot — not exhaustive; the live catalog
adds newer models such as `claude-opus-4-8` and `claude-fable-5`):**

| Model ID | Context Window | Max Output | Input ($/1M) | Output ($/1M) |
|---|---|---|---|---|
| `claude-opus-4-7` | 1,000,000 | 128,000 | $5.00 | $25.00 |
| `claude-opus-4-6` | 1,000,000 | 128,000 | $5.00 | $25.00 |
| `claude-sonnet-4-6` | 1,000,000 | 64,000 | $3.00 | $15.00 |
| `claude-haiku-4-5` | 200,000 | 64,000 | $1.00 | $5.00 |

Run `coven-code models anthropic` for the full, up-to-date list with pricing.
All Anthropic 4.x models support tool calling, vision, and extended reasoning.

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

**Authentication:** Sign in with `coven-code codex login` (ChatGPT/Codex OAuth). Tokens are stored per profile under `~/.coven-code/accounts/codex/<id>/`. See [auth.md](auth.md) for the multi-account flow.

**Default model:** `gpt-5.2-codex`

**Configuration:**

```json
{
  "provider": "codex"
}
```

Codex credentials are managed entirely through the OAuth login; you do not place a key in `settings.json`.

---

## Per-Provider Configuration in settings.json

The `providers` map in `~/.coven-code/settings.json` accepts per-provider `ProviderConfig` objects:

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
