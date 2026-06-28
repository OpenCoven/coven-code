#!/usr/bin/env bash
# shellcheck shell=bash
#
# Prompt input: text echoes into the buffer, and Ctrl+U clears it.

register_case tc_input

tc_input() {
  describe "Prompt input editing"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  local marker="zzqx_typed_marker"
  tui_type "$marker"
  if wait_for "$marker" 5; then
    _pass "typed text appears in the input buffer"
  else
    _fail "typed text appears in the input buffer" "$(tui_capture)"
  fi

  # Ctrl+U clears the line (verified binding).
  tui_keys C-u
  if wait_absent "$marker" 5; then
    _pass "Ctrl+U clears the input buffer"
  else
    _fail "Ctrl+U clears the input buffer" "$(tui_capture)"
  fi

  tui_stop
}
