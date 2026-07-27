# Suggested-reply autofill — design

**Date:** 2026-07-11
**Status:** approved (design review in chat; default-on confirmed by user)

## Overview

After each completed assistant turn, the TUI offers a recommended next user
message as dim ghost text in the (empty) prompt input. Pressing Tab accepts it
— the text fills the input for editing or submission; it is never auto-sent.
Typing anything else replaces it. The recommendation is authored by the main
model itself, inline in the turn, so the feature adds no extra API calls and
works identically across both providers (Claude via API or CLI transport, and
Codex).

## Non-goals

- Auto-sending the suggested reply.
- Multiple ranked suggestions.
- Suggestions from a separate side-model call or local heuristics (considered
  and rejected: per-turn cost/latency, awkward through the claude-CLI
  subscription transport; heuristics too generic).

## Architecture

### 1. Generation (crates/core)

`system_prompt.rs` gains a short addendum, emitted only when the
`suggested_reply` setting is enabled:

> After completing your response, if there is an obvious next user message
> (confirming a proposed action, picking an option, a natural follow-up),
> append exactly one tag on its own line as the very last thing you output:
> `<suggested-reply>the reply</suggested-reply>`. Keep it under 120
> characters, plain text, written in the user's voice (e.g. "yes, apply the
> fix", "commit it", "/review"). If no sensible reply exists, append nothing.

New setting in `claurst_core::Settings` (`crates/core/src/lib.rs`):
`suggested_reply: bool`, **default `true`**, documented in
`docs/configuration.md`.

### 2. Extraction (crates/tui)

- A parser function (unit-testable, in `prompt_input.rs`, which already owns
  the input/typeahead parsing helpers) takes the final assistant text and returns
  `(display_text, Option<String>)` — the text with one *trailing*
  `<suggested-reply>…</suggested-reply>` tag removed, and the tag's content.
  A tag anywhere other than the tail of the message is left untouched.
- Called from `App::flush_streamed_assistant_message` (`app.rs:1838`) before
  the message is pushed to the transcript; the extracted suggestion is stored
  on `App`.
- **Streaming hold-back:** the live streaming display must not flash the tag
  while it arrives. The streaming render path withholds a trailing partial
  match of `<suggested-reply` (or a complete-but-unclosed tag) from the
  visible tail of `streaming_text`. The underlying buffer is unchanged; only
  the rendered tail is trimmed. Chunk boundaries splitting the tag mid-token
  must be covered by tests.
- Sanitization: suggestion is trimmed, control characters stripped, and
  capped at 200 characters (over-cap → discarded, not truncated).

### 3. State & UX (crates/tui)

- `App` stores `suggested_reply: Option<String>`. Extraction stashes the
  candidate during `flush_streamed_assistant_message` (which the
  `QueryEvent::TurnComplete` handler at `app.rs:6709` invokes); the handler
  then keeps it only if all hold:
  - the turn was not cancelled and did not error,
  - the prompt input is empty,
  - `queued_messages` is empty,
  - no overlay/dialog is open.
- Render: when the prompt input is empty and a suggestion is present, draw
  the suggestion inline as dim (DarkGray) ghost text with a trailing
  `⇥ accept` hint. Any character input clears the ghost immediately.
- Accept (Tab): copies the suggestion into the prompt input (cursor at end)
  and clears the ghost. Submission stays a separate, explicit Enter.
- Cleared on: accept, any typed input, starting a new turn, `/clear`,
  session switch/resume.

### 4. Keybinding (crates/core)

Per the repo keybinding rule, no inline key checks. New action
`acceptSuggestion` with default chord `tab` in `KeyContext::Chat`,
added to `crates/core/src/keybindings.rs`. Dispatch order: when ghost text
is visible, `acceptSuggestion` consumes Tab; otherwise the existing Tab
behavior (mode cycling / indent) is untouched.

### 5. Config & docs

- `/config suggested-reply on|off` subcommand in the ConfigCommand
  (`crates/commands/src/lib.rs`), mirroring existing boolean toggles.
- Documented in `docs/configuration.md` (setting) and `docs/commands.md`
  (`/config` section).

## Data flow

```
model output (ends with <suggested-reply>…</suggested-reply>)
  → streaming render (tail hold-back keeps tag invisible)
  → flush_streamed_assistant_message: extract tag
      → transcript gets clean text
      → App.suggested_reply = Some(content)
  → TurnComplete: eligibility checks (cancelled? input empty? queue empty?)
  → render: ghost text in empty prompt input
  → Tab (acceptSuggestion) → input filled, ghost cleared
  → user edits/submits as normal
```

## Error handling

- No tag, malformed tag, empty or over-cap content → no suggestion; the
  feature is silently idle for that turn. Never an error surface.
- Tag not at the message tail → treated as literal text (defends against
  quoted examples in prose).
- Setting off → no system-prompt addendum *and* extraction still strips a
  trailing tag if a model emits one anyway (belt and suspenders; nothing is
  shown).

## Testing

- Extraction: complete tag, absent, unclosed, mid-text (untouched), split
  across stream chunk boundaries, over-cap content, control characters.
- Hold-back: rendered streaming tail never contains `<suggested-reply`
  fragments for any chunking of a tagged message.
- State: suppression under each eligibility condition; cleared on
  accept/type/new-turn/clear.
- Render: TestBackend test — ghost text drawn dim when input empty +
  suggestion present; absent after a keypress; absent when disabled.
- Keybinding: Tab accepts only while ghost visible; mode-cycling Tab
  unaffected otherwise.

## Files touched

| File | Change |
|---|---|
| `crates/core/src/lib.rs` | `suggested_reply` setting (default true) |
| `crates/core/src/system_prompt.rs` | conditional addendum |
| `crates/core/src/keybindings.rs` | `acceptSuggestion` action, default `tab` |
| `crates/tui/src/prompt_input.rs` | extraction fn, ghost-text state/render |
| `crates/tui/src/app.rs` | flush-time extraction, TurnComplete eligibility, dispatch |
| `crates/tui/src/render.rs` | ghost-text drawing in the input line |
| `crates/commands/src/lib.rs` | `/config suggested-reply` toggle |
| `docs/configuration.md`, `docs/commands.md` | documentation |
