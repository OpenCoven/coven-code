---
name: github
description: Use GitHub context without leaking tokens or taking repository state changes without approval.
---

# GitHub

Use GitHub context without leaking tokens or taking repository state changes without approval.

## Use When
- Inspect issues and pull requests
- Read repository history
- Prepare review or release context

## Guardrails
- Do not push, merge, close, label, or comment without explicit approval
- Prefer read-only queries before mutations
- Never reveal token values

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
