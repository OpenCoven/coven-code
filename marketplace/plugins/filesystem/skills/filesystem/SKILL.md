---
name: filesystem
description: Use filesystem MCP only with a narrow allowed root and explicit write intent.
---

# Filesystem

Use filesystem MCP only with a narrow allowed root and explicit write intent.

## Use When
- Read scoped project files
- Inspect local artifacts
- Apply approved file edits

## Guardrails
- Use the smallest useful root
- Do not delete files without approval
- Do not expose secrets from local files

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
