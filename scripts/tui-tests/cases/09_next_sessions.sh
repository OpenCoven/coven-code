#!/usr/bin/env bash
# shellcheck shell=bash
#
# Next session navigation (#412): two sessions in one disposable project,
# resumed from the session browser with Enter, with honest ownership in the
# header. A fake `claude` answers each turn at once; no network or model.
# This proves browser resume and the ownership label only. It is not
# evidence for daemon reconnect, identity binding or protected actions.

register_case tc_next_sessions

_next_sessions_fake_claude() {
  cat >"$1" <<'FAKE'
#!/bin/sh
# Fake `claude`: answer once and finish.
cat >/dev/null &
printf '%s\n' '{"type":"system","subtype":"init","session_id":"fake","model":"fake-model"}'
printf '%s\n' '{"type":"assistant","message":{"model":"fake-model","content":[{"type":"text","text":"fake answer"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":3,"output_tokens":1}}'
FAKE
  chmod +x "$1"
}

_next_sessions_turn() {
  tui_type "$1"; tui_keys Enter
  if wait_for "fake answer" 15; then _pass "$2"
  else _fail "$2" "$(tui_capture)"; fi
}

tc_next_sessions() {
  describe "Next session navigation (COVEN_CODE_UI=next)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/bin" "$scratch/acme-sessions"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  _next_sessions_fake_claude "$scratch/bin/claude"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-sessions") && env"
  local var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next \
    "COVEN_CODE_CLAUDE_BIN=$scratch/bin/claude")"
  TUI_BOOT_STRING="preview"

  # Session A: one turn, then quit so it is saved.
  if tui_start; then
    _next_sessions_turn "alpha question" "session A answers"
    assert_contains "$(tui_capture)" "engine-local" "the header says who runs the session"
    assert_absent "$(tui_capture)" "daemon-managed" "nothing claims daemon control"
    tui_keys C-c; sleep 0.3; tui_keys C-c
    sleep 1
  fi
  tui_stop

  # Session B: a new session in the same project resumes A from the browser.
  if tui_start; then
    _next_sessions_turn "beta question" "session B answers"
    tui_type "/session resume"; tui_keys Enter
    if wait_for "Last Updated" 10; then _pass "the browser opens"
    else _fail "the browser opens" "$(tui_capture)"; fi
    sleep 1
    local rows; rows="$(tui_capture | grep -c "just now")"
    if [ "$rows" -ge 2 ]; then _pass "the browser lists both saved sessions"
    else _fail "the browser lists both saved sessions" "$(tui_capture)"; fi
    # Newest first: B (current) is on top, A below it.
    tui_keys Down; sleep 0.2
    tui_keys Enter
    if wait_for "Resumed session" 10; then _pass "Enter in the browser resumes the session"
    else _fail "Enter in the browser resumes the session" "$(tui_capture)"; fi
    local s; s="$(tui_capture)"
    assert_absent "$s" "beta question" "session B's transcript is gone after the switch"
    assert_contains "$s" "alpha question" "session A's transcript is shown"
    tui_keys C-c; sleep 0.3; tui_keys C-c
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}
