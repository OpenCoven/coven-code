---
name: gmail
description: Use Gmail for triage and drafting while keeping external sends behind explicit approval.
---

# Gmail

Use Gmail for triage and drafting while keeping external sends behind explicit approval.

## Use When
- Summarize unread or searched mail
- Prepare reply drafts
- Extract action items into plans or tasks

## Guardrails
- Do not send mail without explicit approval
- Quote only what is needed
- Treat email content as private by default

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
