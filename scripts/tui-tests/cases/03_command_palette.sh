#!/usr/bin/env bash
# shellcheck shell=bash
#
# Slash command palette: opening, filtering, and Ctrl+K entry point.

register_case tc_palette

tc_palette() {
  describe "Command palette"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  # Typing "/" as the first character opens the palette.
  tui_type "/"
  if ! wait_for "/config"; then
    _fail "palette opened on '/' (/config never appeared)" "$(tui_capture)"
    tui_stop; return 0
  fi
  local s; s="$(tui_capture)"
  assert_contains "$s" "/clear"   "palette lists /clear"
  assert_contains "$s" "/compact" "palette lists /compact"
  assert_contains "$s" "/config"  "palette lists /config"

  # Filtering narrows the list: "/config" should keep /config, drop /clear.
  tui_type "config"
  # Wait for the filter to take effect (a non-matching entry disappears).
  wait_absent "/clear" 5
  s="$(tui_capture)"
  assert_contains "$s" "/config" "filter '/config' keeps /config"
  assert_absent   "$s" "/clear"  "filter '/config' drops /clear"

  # Esc dismisses the palette; Ctrl+U clears the leftover input.
  tui_keys Escape; tui_settle
  tui_keys C-u; tui_settle

  # Ctrl+K is the second documented entry point to the palette.
  tui_keys C-k
  if wait_for "/clear" 5; then
    _pass "Ctrl+K opens the command palette"
  else
    _fail "Ctrl+K opens the command palette" "$(tui_capture)"
  fi
  tui_keys Escape; tui_settle

  tui_stop
}
