---
name: asana
description: Use Asana for work graph lookup, task summaries, and project status without silently changing assignments.
---

# Asana

Use Asana for work graph lookup, task summaries, and project status without silently changing assignments.

## Use When
- Find incomplete tasks
- Summarize project status
- Draft task updates

## Guardrails
- Do not create, assign, complete, or delete tasks without approval
- Keep work status concrete
- Avoid copying private planning notes into shared workspaces

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
