---
name: vercel
description: Use Vercel deployment and docs context for debugging and release planning.
---

# Vercel

Use Vercel deployment and docs context for debugging and release planning.

## Use When
- Inspect deployment status
- Read runtime logs
- Look up Vercel docs from the official MCP server

## Guardrails
- Do not promote, rollback, or change env vars without approval
- Treat logs as potentially sensitive
- Prefer project-scoped queries

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
