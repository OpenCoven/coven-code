#!/usr/bin/env bash
# shellcheck shell=bash
#
# Shutdown: Ctrl+C twice exits the TUI and returns the shell.

register_case tc_quit

tc_quit() {
  describe "Quit / shutdown"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  # First Ctrl+C is "cancel"; the second confirms quit.
  tui_keys C-c; sleep 0.4
  tui_keys C-c

  # Prove the process actually exited by dropping a marker on the shell that
  # is revealed once the TUI tears down.
  if wait_for "Coven Code v" 2; then
    : # still showing — fall through to explicit check below
  fi
  tui_type "echo __TUI_EXITED__"; tui_keys Enter
  if wait_for "__TUI_EXITED__" 5; then
    _pass "Ctrl+C twice exits to the shell"
  else
    _fail "Ctrl+C twice exits to the shell" "$(tui_capture)"
  fi

  tui_stop
}
