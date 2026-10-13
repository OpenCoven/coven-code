#!/usr/bin/env bash
# shellcheck shell=bash
#
# Slash command palette: opening, filtering, and Ctrl+K entry point.

register_case tc_palette

tc_palette() {
  describe "Command palette"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  # Contextual roots replace the expired standalone account/integration paths.
  tui_type "/"
  if ! wait_for "/help"; then
    _fail "palette opened on '/' (/help never appeared)" "$(tui_capture)"
    tui_stop; return 0
  fi
  local s; s="$(tui_capture)"
  assert_contains "$s" "/help" "palette lists contextual /help"
  assert_absent "$s" "/accounts " "palette omits retired /accounts"
  assert_absent "$s" "/attach " "palette omits retired /attach"
  assert_absent "$s" "/chrome " "palette omits retired /chrome"

  # Filtering narrows the list: "/config" should surface /config, drop /clear.
  tui_type "config"
  # The contextual starting list may already omit /clear. Wait for the
  # requested result instead, proving the new filtered frame has rendered.
  wait_for "/config" 5
  s="$(tui_capture)"
  assert_contains "$s" "/config" "filter '/config' keeps /config"
  assert_absent   "$s" "/clear"  "filter '/config' drops /clear"

  # Esc dismisses the palette; Ctrl+U clears the leftover input.
  tui_keys Escape; tui_settle
  tui_keys C-u; tui_settle

  # Ctrl+K is the second documented entry point to the palette.
  tui_keys C-k
  if wait_for "Command Palette" 5; then
    _pass "Ctrl+K opens the command palette"
  else
    _fail "Ctrl+K opens the command palette" "$(tui_capture)"
  fi
  tui_keys Escape; tui_settle

  # Enter on a selected palette leaf submits through the CLI in one step.
  tui_keys C-k; tui_settle
  tui_type "help feedback"; tui_settle
  tui_keys Enter
  if wait_for "Session Feedback" 5; then
    _pass "palette leaf executes on one Enter"
  else
    _fail "palette leaf executes on one Enter" "$(tui_capture)"
  fi

  tui_keys Escape; tui_settle

  # Exact retired input must be rejected before automatic suggestion acceptance.
  tui_type "/fork"; tui_settle; tui_keys Enter
  if wait_for "/fork was removed; use /session fork" 5; then
    _pass "retired /fork cannot execute through completion"
  else
    _fail "retired /fork cannot execute through completion" "$(tui_capture)"
  fi
  tui_type "/theme"; tui_settle; tui_keys Enter
  if wait_for "/theme was removed; use /config theme" 5; then
    _pass "retired /theme cannot open picker through completion"
  else
    _fail "retired /theme cannot open picker through completion" "$(tui_capture)"
  fi

  tui_stop
}
