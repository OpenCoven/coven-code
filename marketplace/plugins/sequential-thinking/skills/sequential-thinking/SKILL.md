---
name: sequential-thinking
description: Use Sequential Thinking for complex reasoning, not as a substitute for evidence or tests.
---

# Sequential Thinking

Use Sequential Thinking for complex reasoning, not as a substitute for evidence or tests.

## Use When
- Break down ambiguous tasks
- Trace debugging hypotheses
- Compare options before a recommendation

## Guardrails
- Do not present private reasoning as final output
- Verify claims with tools or sources
- Stop when the path is clear enough to act

## Default Flow

1. Confirm the user intent and whether the action is read-only or state-changing.
2. Use the narrowest available tool scope and collect only the context needed for the task.
3. For state-changing or external actions, stop for explicit approval before acting.
4. Summarize what changed or what was learned, including relevant object IDs or links.
