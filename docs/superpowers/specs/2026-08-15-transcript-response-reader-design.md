# Transcript response reader

## Purpose

Completed assistant messages can be much taller than the terminal transcript
viewport. Their content is currently retained, but the only discovery path is
the transcript scroller. Add a focused reader so a completed answer is clearly
available without turning ordinary chat into a document viewer.

## Scope

- Improve the interactive Rust TUI transcript for completed assistant answers.
- Keep streamed output inline and keep the current transcript as the default
  surface.
- Add an explicit, keyboard-first full-response reader.
- Preserve existing markdown rendering and message contents; do not alter
  provider streaming, persistence, session format, tool execution, or exports.

## Transcript card

Every completed assistant response receives a compact completion footer:

- `Response · N lines` where `N` is the rendered visual-line count.
- `Enter to read` as the primary affordance.
- Existing completion timing and collapsed tool summaries remain visible.

The transcript continues to render the complete response normally. The footer
does not truncate or replace it; it makes the full-reader path discoverable.
The existing scrollbar remains the normal way to browse transcript history.

## Reader interaction

`Enter` opens the reader for the selected completed assistant response. If no
response is selected, the command applies to the latest completed assistant
response. The reader contains:

- the full markdown-rendered assistant text;
- a stable position indicator (`line X / N` or equivalent);
- `PgUp`/`PgDn`, `j`/`k`, and `Home`/`End` navigation;
- in-reader text search and copy;
- `Esc` to return to the transcript at the same response and scroll position.

Tool calls stay collapsed in the reader. The reader is a response-reading
surface, not a second execution history.

## State and behavior

- The reader is available for every completed assistant response, regardless
  of length. This keeps the interaction predictable; the visible line count
  communicates when it is useful.
- It never opens automatically when a turn finishes or when a background task
  completes.
- Starting a new user turn closes the reader and returns focus to the prompt.
- While a turn is streaming, no reader affordance is shown for its unfinished
  text. Completed prior responses remain readable.
- Empty assistant messages and assistant messages containing only tool calls
  do not offer the reader.

## Architecture

Add a TUI-local `ResponseReaderState` that owns only:

- the target message index;
- its vertical scroll offset;
- query/search state; and
- the transcript scroll position to restore on exit.

The reader obtains content from the existing in-memory `App.messages` entry.
It reuses `render_markdown` and does not duplicate response text or modify the
persisted `ConversationSession`. Rendering and input handling are isolated in
a dedicated reader module, with the app routing commands based on reader
visibility.

## Validation

Add focused TUI tests that prove:

1. A 6,000+ character completed assistant response retains all text in the
   normal transcript render.
2. The transcript footer exposes the rendered line count and reader affordance.
3. Opening the reader targets the expected assistant message and renders the
   complete response.
4. Reader navigation changes only reader scroll state.
5. `Esc` restores the prior transcript position; a new prompt exits reader
   mode and submits normally.
6. Tool-only/empty assistant messages do not offer a reader.

## Non-goals

- Auto-opening answers, response summarization, content truncation, or a new
  persistence format.
- Changing provider output, model behavior, or streamed-event handling.
- Replacing the transcript scrollbar with a separate response navigator.
