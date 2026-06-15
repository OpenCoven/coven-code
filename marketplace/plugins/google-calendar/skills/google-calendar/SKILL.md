---
name: google-calendar
description: Use calendar context for scheduling and planning without creating or changing events silently.
---

# Google Calendar

Use calendar context for scheduling and planning without creating or changing events silently.

## Use When
- Check availability
- Summarize upcoming events
- Draft event details for approval

## Guardrails
- Do not create, move, delete, or invite without explicit approval
- Use concrete dates and time zones
- Minimize disclosure of private event details

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
