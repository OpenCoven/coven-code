---
name: fetch
description: Use Fetch for bounded source retrieval with primary-source preference.
---

# Fetch

Use Fetch for bounded source retrieval with primary-source preference.

## Use When
- Fetch docs pages
- Read articles for synthesis
- Capture cited source context

## Guardrails
- Prefer primary sources
- Do not over-quote copyrighted text
- Record URLs used in summaries

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
