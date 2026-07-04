# TUI UI/UX, Responsiveness & Accessibility Review

A comprehensive review of the coven-code terminal UI (`src-rust/crates/tui/`) across
three dimensions — **responsive rendering (all terminal sizes)**, **accessibility**,
and **UI/UX consistency / seamless integration** — with the fixes applied in this
pass and prioritized follow-ups.

"Screen size" for a TUI means terminal **columns × rows**: tiny (e.g. 20×6), narrow
(~60 cols), short (few rows), and very wide/tall.

---

## Summary

The codebase already has solid accessibility *infrastructure* — a `theme_colors.rs`
with a deuteranopia (colorblind) palette and WCAG-contrast unit tests, effort levels
encoded by **shape** not just color (`figures.rs`), and a small-terminal fallback for
the welcome box. The gaps were: a **live mojibake rendering bug**, selection/focus
conveyed by background color alone, one genuinely color-only status, a handful of
unguarded layout subtractions that can underflow on tiny terminals, and a duplicated
accent color literal that undercuts theming.

---

## Fixed in this pass

### Accessibility

1. **Mojibake in rendered strings (High — live bug on every terminal).**
   `render.rs` contained double-encoded UTF-8 (UTF-8 bytes misread as CP1252 then
   re-encoded). The statusline segment separator rendered as literal `â"‚` instead of
   `│`, the context warning as `âš ` instead of `⚠`, and the bridge glyph as `ðŸ"—`
   instead of `🔗`. Fixed all 12 occurrences (5 rendered + 7 comments) via a safe
   CP1252 round-trip that only touches CP1252-encodable runs, leaving legitimate
   multibyte glyphs (`✓ 🌿 ─ │ ⚠`) untouched.

2. **Focus/selection survives NO_COLOR & monochrome (High).** Selected list rows in
   `dialog_select.rs` and `model_picker.rs` were distinguished **only** by a highlight
   background. Added a `> ` caret marker + `Modifier::BOLD` so the focused item is
   still identifiable when the background can't render (NO_COLOR, monochrome terminal,
   low-contrast theme, colorblind users). `model_picker` reuses its existing 3-char
   leading column, so there's no layout shift. (`ask_user_dialog.rs` already did this
   correctly with a `▶ ` prefix + bold.)

3. **Context-window level is no longer color-only (High).** The statusline context
   gauge showed `Nk/Mk (P%)` colored green/yellow/red — the *number* was visible but
   the *danger level* was carried by color alone. Added a shape prefix (`○` ok / `◐`
   warning / `●` critical, matching the effort-level shape convention) so colorblind
   and monochrome users can read the level.

### Responsiveness (all screen sizes)

4. **Underflow-safe layout math (Medium).** Six unguarded `rect.height - N` /
   `rect.width - N` subtractions could underflow and panic on tiny terminals. Converted
   to `saturating_sub` in `ask_user_dialog.rs` (×2), `prompt_input.rs`, `stats_dialog.rs`,
   and `tasks_overlay.rs`. (`diff_viewer.rs:859` was already guarded by `inner.width > 1`
   and left as-is; `stats_dialog.rs:614` is bounded by its loop and left as-is.)

### UI/UX consistency

5. **Accent color consolidated (Medium).** The violet accent `Color::Rgb(139,92,246)`
   was copy-pasted **21 times across 11 files**. A single source of truth already
   existed — `overlays::COVEN_CODE_ACCENT` — so 18 inline literals were replaced with
   it (leaving the `overlays.rs` definition and the `theme_colors.rs` palette values).
   This is the foundation for making the accent theme-aware.

---

## Verified NOT an issue (checked, no change needed)

- **PR badge & agent-progress status** were flagged as possibly color-only, but both
  render the status as **text** (`[approved]`, `working`/`done`/`error`) — color only
  reinforces. No change.
- **Welcome box** already collapses to a single status line below a minimum size
  (`render.rs` `render_welcome_box`), and the familiar switcher caps its popup to the
  available area with `.min(...).max(4)`.

---

## Prioritized follow-ups (not done here — larger/riskier)

These are documented for a future pass; each is a broader change than this review's
surgical scope.

1. **Honor `NO_COLOR` (High).** The env var is never read anywhere in the workspace.
   A startup check that forces a monochrome style path (drop `.fg()/.bg()`, rely on
   `BOLD`/`REVERSED`) would make the app usable on strictly monochrome setups. The
   focus-marker work above is a prerequisite that's now in place.

2. **Wire the theme system into the main render path (High).** `theme_colors.rs`
   `ColorPalette` / `get_error_color` / `get_success_color` have **no consumers**
   outside the file; only the diff viewer is theme-aware (`diff_viewer.rs:40`). The
   ~53 hardcoded `Rgb` and ~60 `DarkGray` in `render.rs` bypass the selected theme, so
   the deuteranopia theme currently has no effect on the main UI. Route them through
   `ColorPalette::for_theme(&theme_name)`, replicating the diff-viewer pattern.

3. **Raise the worst contrast offenders above ~4.5:1 (High).** Near-invisible dim text:
   `onboarding_dialog.rs` separators (`Rgb(45,45,55)` ≈ 1.45:1), `model_picker.rs:758`
   (`Rgb(40,40,45)` ≈ 1.35:1). Move toward the AA-tested `COVEN_CODE_MUTED`
   (`Rgb(161,161,170)` ≈ 7.7:1). (Verify the stats-chart empty-block dimness is
   intentional before changing it.)

4. **Replace pervasive `Color::DarkGray` with `COVEN_CODE_MUTED` (Medium).** `DarkGray`
   maps to ANSI bright-black, whose RGB is terminal-theme-dependent and often fails
   contrast. A defined muted constant is both consistent and covered by the existing
   WCAG tests.

5. **Unicode capability gate + ASCII fallbacks (Medium).** `figures.rs` has rich
   glyphs with only a Windows/Unix fallback for one symbol and no `LANG`/`LC_*`
   detection. Provide ASCII alternates for glyphs and box-drawing separators for
   ASCII-only / `LANG=C` terminals and log captures.

---

## Test coverage

- `cargo test` across touched crates (see PR).
- `scripts/tui-tests/` interactive tmux suite run at multiple terminal sizes
  (small / narrow / default) to confirm no rendering regressions.
