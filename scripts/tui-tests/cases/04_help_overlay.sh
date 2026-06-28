#!/usr/bin/env bash
# shellcheck shell=bash
#
# Help overlay: "?" opens the keybinding + command reference, Esc closes it.

register_case tc_help

tc_help() {
  describe "Help overlay"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  tui_keys "?"
  # The overlay is tall; under load the lower rows render a beat after the
  # top. Poll for a late item before snapshotting so the assertions don't
  # race the draw.
  if ! wait_for "/permissions"; then
    _fail "help overlay rendered (/permissions never appeared)" "$(tui_capture)"
    tui_stop; return 0
  fi
  local s; s="$(tui_capture)"
  assert_contains "$s" "Toggle help"     "help overlay documents the help toggle"
  assert_contains "$s" "Command palette" "help overlay documents the command palette"
  assert_contains "$s" "Model picker"    "help overlay documents the model picker"
  # Command reference section.
  assert_contains "$s" "/login"          "help overlay lists /login"
  assert_contains "$s" "/permissions"    "help overlay lists /permissions"

  # Esc closes the overlay.
  tui_keys Escape
  if wait_absent "Toggle help"; then
    _pass "Esc closes the help overlay"
  else
    _fail "Esc closes the help overlay" "$(tui_capture)"
  fi

  tui_stop
}
