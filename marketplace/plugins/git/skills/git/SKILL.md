---
name: git
description: Use Git MCP for repository understanding while preserving repo safety rules.
---

# Git

Use Git MCP for repository understanding while preserving repo safety rules.

## Use When
- Inspect diffs
- Read commit history
- Map changed files

## Guardrails
- Do not reset, clean, force-push, or commit through MCP without approval
- Report dirty worktrees before changing state
- Stage only files changed in the current session

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
