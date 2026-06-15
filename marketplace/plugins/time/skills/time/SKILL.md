---
name: time
description: Use Time MCP whenever relative dates, time zones, or scheduling precision matter.
---

# Time

Use Time MCP whenever relative dates, time zones, or scheduling precision matter.

## Use When
- Convert time zones
- Resolve today/tomorrow/yesterday
- Prepare dated status updates

## Guardrails
- Use absolute dates when the user may be confused
- Preserve the user's timezone preference
- Do not schedule external events without approval

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
