---
name: memory
description: Use Memory MCP for explicit, curated recall rather than dumping raw private context.
---

# Memory

Use Memory MCP for explicit, curated recall rather than dumping raw private context.

## Use When
- Create durable facts
- Search prior decisions
- Maintain relationship between people, projects, and events

## Guardrails
- Do not store secrets
- Prefer concise facts over raw transcripts
- Mark uncertainty when memory is inferred

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
