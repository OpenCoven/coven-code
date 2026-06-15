# Marketplace

Coven Code treats first-party marketplace packages as ordinary plugin directories, but Coven Cave is the canonical user-facing marketplace surface. The Code catalog mirrors the Cave-owned OpenCoven package set and `scripts/sync-marketplace.py` expands that catalog into:

- `marketplace/plugins/<name>/plugin.json`
- `marketplace/plugins/<name>/skills/<name>/SKILL.md`
- `marketplace/plugins/<name>/.codex-plugin/plugin.json`
- `marketplace/marketplace.json`
- `marketplace/exports/codex/marketplace.json`
- `marketplace/exports/mcp/mcp.json`
- `marketplace/exports/roles/role-affinity.json`

## Design

The Coven Code plugin directory is the terminal/client compatibility package format because it already supports metadata, skills, user config, and inline `mcpServers` in one installable directory. Compatibility exports are generated from the mirrored catalog so Code, Codex, MCP-only clients, and Cave role views do not drift from the Cave-owned package metadata.

Each catalog entry includes:

- package metadata for Coven Code and Codex
- optional MCP server configuration
- user-config declarations for sensitive setup values
- trust level, source references, and role affinity
- one generated Skill that tells familiars how to use the integration safely

## Seed Packages

The first seed starts with integrations already used by Val's familiar lanes:

- GitHub
- Gmail
- Google Calendar
- Linear
- Canva
- Vercel
- Asana
- xurl

It also includes the conservative common MCP starter set from the reference MCP servers:

- Filesystem
- Git
- Fetch
- Memory
- Sequential Thinking
- Time

## Trust Levels

`official-remote` packages point at a service-operated remote MCP endpoint, such as Linear, Vercel, Canva, or Asana.

`reference-local` packages use the MCP reference server catalog. These are useful defaults, but upstream describes them as reference implementations rather than production-ready packages, so installers should still apply local threat-model checks.

`preview-local` packages use a local tool or preview integration whose exact command surface may move before a stable marketplace release.

`local-tool` packages wrap a local OpenCoven/OpenClaw tool that is part of Val's familiar setup rather than an external MCP service.

## Updating

Edit `marketplace/catalog.json`, then run:

```bash
python3 scripts/sync-marketplace.py
python3 scripts/sync-marketplace.py --check
```

The check command fails if generated packages or exports are missing or stale.
