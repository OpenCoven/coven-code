#!/usr/bin/env bash
# shellcheck shell=bash
#
# Resuming a session whose id is not a UUID (#573), in Classic and in Next.
# `--session-id` takes any string, and the history keeps it as given. Two
# sessions are saved, one with the 2-byte id `ab` and one with
# `日本語セッション`, whose byte 8 falls inside its third character. A fresh
# launch resumes each with `/session resume <id>`, which replaced `/resume`.
# The app keeps running and shows the resumed transcript; Next also shows the
# shortened id in its status. A fake
# `claude` answers each turn at once; no network or model. Classic shows the
# status row only before its first turn, so there only the transcript and the
# running process are checked.

register_case tc_resume_short_session_id

_rss_fake_claude() {
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

# One saved session: start with the id, answer one prompt, quit.
_rss_save() { # renderer, id, prompt
  if tui_start --session-id "$2"; then
    tui_type "$3"; tui_keys Enter
    if wait_for "fake answer" 15; then _pass "$1: session $2 answers"
    else _fail "$1: session $2 answers" "$(tui_capture)"; fi
    tui_keys C-c; sleep 0.3; tui_keys C-c
    sleep 1
  fi
  tui_stop
}

# Resume one saved session in a fresh launch, so each id is tried on a
# running app.
_rss_resume() { # renderer, id, prompt of that session
  local r="$1" id="$2" prompt="$3" running
  if ! tui_start; then return 0; fi
  running="$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')"
  tui_type "/session resume $id"; sleep 0.3; tui_keys Enter
  if wait_for "$prompt" 10; then _pass "$r: /session resume $id shows that session's transcript"
  else _fail "$r: /session resume $id shows that session's transcript" "$(tui_capture)"; fi
  sleep 0.5
  assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" \
    "$running" "$r: the app is still running after /session resume $id"
  assert_absent "$(tui_capture)" "panicked" "$r: /session resume $id does not panic"
  if [ "$r" = next ]; then
    if wait_for "Resumed session $id." 5; then _pass "$r: the status names session $id"
    else _fail "$r: the status names session $id" "$(tui_capture)"; fi
  fi
  tui_stop
}

_rss_journey() { # renderer
  local r="$1" scratch
  scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/bin" "$scratch/project"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  _rss_fake_claude "$scratch/bin/claude"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" var ui=""
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/project") && env"
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT \
    COVEN_DAEMON_SOCKET COVEN_CODE_UI; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  if [ "$r" = next ]; then ui="COVEN_CODE_UI=next"; TUI_BOOT_STRING="preview"
  else TUI_BOOT_STRING="Coven v"; fi
  # shellcheck disable=SC2086 # an empty selector adds nothing
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" \
    "COVEN_CODE_CLAUDE_BIN=$scratch/bin/claude" $ui)"

  _rss_save "$r" ab "alpha question"
  _rss_save "$r" "日本語セッション" "beta question"

  _rss_resume "$r" ab "alpha question"
  _rss_resume "$r" "日本語セッション" "beta question"

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

tc_resume_short_session_id() {
  describe "/session resume of a short or non-ASCII session id does not panic"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  _rss_journey classic
  _rss_journey next
}
