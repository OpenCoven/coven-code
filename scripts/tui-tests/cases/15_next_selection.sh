#!/usr/bin/env bash
# shellcheck shell=bash
#
# Next selection journey (#410): drag a real SGR mouse selection, resize the
# terminal, and check the mark does not land on other text. Copying is proven
# in crates/tui/tests/next_shell.rs; this case never presses Ctrl+C, so it
# cannot write to the clipboard of the machine running it. Auto-copy is off by
# default and the home is disposable.

register_case tc_next_selection

_next_selection_fake_claude() {
  cat >"$1" <<'FAKE'
#!/bin/sh
# Fake `claude`: one 30-line answer, then a finished turn.
cat >/dev/null &
printf '%s\n' '{"type":"system","subtype":"init","session_id":"fake","model":"fake-model"}'
i=1; text=""
while [ "$i" -le 30 ]; do text="${text}alpha line $i\\n"; i=$((i + 1)); done
printf '%s\n' "{\"type\":\"assistant\",\"message\":{\"model\":\"fake-model\",\"content\":[{\"type\":\"text\",\"text\":\"$text\"}]}}"
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":3,"output_tokens":5}}'
FAKE
  chmod +x "$1"
}

# The mark Next's transcript selection uses, as tmux reports it.
_next_selection_marked() {
  _tmux capture-pane -t "$TUI_SESSION" -e -p 2>/dev/null | grep -c '48;2;200;200;200' || true
}

tc_next_selection() {
  describe "Next selection across resize (COVEN_CODE_UI=next)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/bin" "$scratch/acme-next"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  _next_selection_fake_claude "$scratch/bin/claude"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  local saved_width="$TUI_WIDTH" saved_height="$TUI_HEIGHT"
  # Scrub anything that could reach a real provider or engine home.
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-next") && env"
  local var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next \
    "COVEN_CODE_CLAUDE_BIN=$scratch/bin/claude")"
  TUI_BOOT_STRING="preview"
  TUI_WIDTH=100 TUI_HEIGHT=30

  if tui_start; then
    _next_selection_journey
  fi

  tui_stop
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  TUI_WIDTH="$saved_width" TUI_HEIGHT="$saved_height"
  rm -rf "$scratch"
}

_next_selection_journey() {
  tui_type "show lines"; tui_keys Enter
  if ! wait_for "alpha line 30" 15; then
    _fail "the answer did not arrive" "$(tui_capture)"
    return 0
  fi
  wait_for "ready" 10 >/dev/null || true

  local row column
  row="$(tui_capture | grep -n 'alpha line 20' | head -1 | cut -d: -f1)"
  column="$(tui_capture | grep 'alpha line 20' | head -1 | awk '{ print index($0, "alpha line 20") }')"
  if [ -z "$row" ] || [ -z "$column" ]; then
    _fail "the line to select is on screen" "$(tui_capture)"
    return 0
  fi

  # Press, drag and release, in SGR mouse coordinates (1-based).
  local end=$((column + 12))
  tui_type "$(printf '\033[<0;%d;%dM' "$column" "$row")"
  tui_type "$(printf '\033[<32;%d;%dM' "$end" "$row")"
  tui_type "$(printf '\033[<0;%d;%dm' "$end" "$row")"
  tui_settle 0.6
  assert_eq "$(_next_selection_marked)" "1" "the dragged text is marked on one row"
  assert_absent "$(tui_capture)" "selection kept" "nothing is kept while the text is on screen"

  _tmux resize-window -t "$TUI_SESSION" -x 80 -y 24
  if wait_for "selection kept" 5; then
    _pass "a resize that moves the text says the selection is kept"
  else
    _fail "a resize that moves the text says the selection is kept" "$(tui_capture)"
  fi
  assert_eq "$(_next_selection_marked)" "0" "no other text is marked after the resize"

  _tmux resize-window -t "$TUI_SESSION" -x 100 -y 30
  tui_settle 0.6
  assert_eq "$(_next_selection_marked)" "0" "resizing back does not guess at the old cells"
  assert_contains "$(tui_capture)" "selection kept" "the selection is still kept"

  tui_type "q7z"
  if wait_absent "selection kept" 5; then
    _pass "typing lets go of the kept selection"
  else
    _fail "typing lets go of the kept selection" "$(tui_capture)"
  fi
  wait_for "q7z" 5 >/dev/null || true
  assert_contains "$(tui_capture)" "q7z" "the keys still reached the draft"
}
