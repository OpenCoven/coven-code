#!/usr/bin/env bash
# shellcheck shell=bash
#
# TUI cold-start: the welcome screen, status line, and footer render.

register_case tc_startup

tc_startup() {
  describe "TUI startup & chrome"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  # tui_start only waits for the frame title; give the welcome card a beat to
  # populate before snapshotting.
  wait_for "What's new" 5
  local s; s="$(tui_capture)"

  assert_contains "$s" "Coven Code v" "title banner renders with version"

  # Right-hand welcome column. Wording is stable; the username is not, so we
  # assert on the static labels only.
  assert_contains "$s" "Tips for getting started" "welcome shows tips section"
  assert_contains "$s" "What's new"               "welcome shows changelog section"

  # Input affordance.
  assert_contains "$s" "❯" "input prompt glyph present"

  # Footer hint bar exposes the core keybindings. The help hint moved out of
  # the footer (9e64bbc dropped Alt+H); /help coverage lives in 04_help_overlay.
  assert_contains "$s" "familiar" "footer advertises familiar binding"
  assert_contains "$s" "branch"   "footer advertises branch binding"
  assert_contains "$s" "mode"     "footer advertises mode binding"

  # No panic / error banner on a clean boot.
  assert_absent "$s" "panicked at"          "no rust panic on startup"
  assert_absent "$s" "RUST_BACKTRACE"       "no backtrace prompt on startup"

  tui_stop
}
