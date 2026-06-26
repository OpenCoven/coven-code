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

**Default model:** `claude-sonnet-4-6`

**Available models (bundled snapshot):**

| Model ID | Context Window | Max Output | Input ($/1M) | Output ($/1M) |
|---|---|---|---|---|
| `claude-opus-4-6` | 200,000 | 32,000 | $15.00 | $75.00 |
| `claude-sonnet-4-6` | 200,000 | 16,000 | $3.00 | $15.00 |
| `claude-haiku-4-5-20251001` | 200,000 | 8,096 | $0.80 | $4.00 |

All Anthropic models support tool calling, vision, and extended reasoning.

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

**Default model:** `gpt-5-codex`

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

Coven Code ships a bundled snapshot of Claude models. When no model is explicitly set, Coven Code scores available models by priority patterns to pick the best default. Well-known `claude-*` model prefixes are always routed to Anthropic.
