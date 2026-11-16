#!/usr/bin/env bash
# shellcheck shell=bash
#
# Help overlay: "?" opens the keybinding + command reference, Esc closes it.
#
# The reference is taller than a small terminal (#423). At 120x40 every item
# below is on the first screen; at 80x24 it is one scrolling column, and the
# case pages down until it has seen each item, which proves the content is
# reachable rather than only that it exists.

register_case tc_help

tc_help() {
  describe "Help overlay"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  tui_keys "?"
  if ! wait_for "Shortcuts & commands"; then
    _fail "help overlay rendered (title never appeared)" "$(tui_capture)"
    tui_stop; return 0
  fi
  local seen="" page needle all
  for page in 1 2 3 4 5 6 7 8 9 10 11 12; do
    tui_settle
    seen+="$(tui_capture)"$'\n'
    all=1
    for needle in "Toggle help" "Command palette" "Model picker" "/session resume"; do
      grep -qF -- "$needle" <<<"$seen" || all=0
    done
    [ "$all" = 1 ] && break
    tui_keys PageDown
  done
  [ "$page" -gt 1 ] && info "help needed $((page - 1)) PageDown(s) at ${TUI_WIDTH}x${TUI_HEIGHT}"
  assert_contains "$seen" "Toggle help"     "help overlay documents the help toggle"
  assert_contains "$seen" "Command palette" "help overlay documents the command palette"
  assert_contains "$seen" "Model picker"    "help overlay documents the model picker"
  # Command reference section.
  assert_contains "$seen" "/session resume" "help overlay lists grouped session commands"
  tui_type "account login"
  if wait_for "/account login" 5; then
    _pass "help filter finds canonical /account login"
  else
    _fail "help filter finds canonical /account login" "$(tui_capture)"
  fi

  # Esc closes the overlay. The filter above already hides the shortcut rows,
  # so wait on the title, which shows for as long as the overlay is open.
  tui_keys Escape
  if wait_absent "Shortcuts & commands"; then
    _pass "Esc closes the help overlay"
  else
    _fail "Esc closes the help overlay" "$(tui_capture)"
  fi

  tui_stop
}
