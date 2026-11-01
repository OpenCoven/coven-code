#!/usr/bin/env bash
# shellcheck shell=bash
#
# Next shell journey (#410): COVEN_CODE_UI=next from a fresh, unconfigured
# home through draft -> inspector -> paste -> submit -> stream -> scroll ->
# resize -> jump to live -> cancel -> quit, then terminal restoration.
#
# The turn runs through the engine's keyless Claude transport against a fake
# `claude` that answers once and then keeps the turn running, so streaming,
# reading position and cancellation are exercised with no network or model.
# The terminal is resized between steps (120x40, 80x24, 40x12, back to
# 120x40). A final Classic segment checks that Escape stops a running turn
# there too. This proves the shell journey only; it is not evidence for tool,
# approval, task, research or memory behavior.

register_case tc_next_shell

_next_shell_fake_claude() {
  cat >"$1" <<'FAKE'
#!/bin/sh
# Fake `claude`: one 60-line answer, then a turn that does not finish.
cat >/dev/null &
printf '%s\n' '{"type":"system","subtype":"init","session_id":"fake","model":"fake-model"}'
i=1; text=""
while [ "$i" -le 60 ]; do text="${text}fake line $i\\n"; i=$((i + 1)); done
printf '%s\n' "{\"type\":\"assistant\",\"message\":{\"model\":\"fake-model\",\"content\":[{\"type\":\"text\",\"text\":\"${text}\"}]}}"
sleep 6
touch "$FAKE_CLAUDE_DONE"
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":3,"output_tokens":1}}'
FAKE
  chmod +x "$1"
}

tc_next_shell() {
  describe "Next shell journey (COVEN_CODE_UI=next)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/bin" "$scratch/acme-next"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  _next_shell_fake_claude "$scratch/bin/claude"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  # Scrub anything that could reach a real provider or engine home.
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-next") && env"
  local var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next \
    "COVEN_CODE_CLAUDE_BIN=$scratch/bin/claude" "FAKE_CLAUDE_DONE=$scratch/done")"
  TUI_BOOT_STRING="preview"

  if tui_start; then
    local s
    s="$(tui_capture)"
    assert_contains "$s" "acme-next" "workspace line names the project"
    assert_contains "$s" "ready" "fresh home opens idle with no provider action"
    assert_absent "$s" "Coven v" "Next draws its own shell, not Classic's welcome"

    # ---- draft and the one inspector ------------------------------------
    _tmux resize-window -t "$TUI_SESSION" -x 80 -y 24
    tui_type "draft one"
    if wait_for "draft not sent" 5; then _pass "a typed draft is visibly unsent"
    else _fail "a typed draft is visibly unsent" "$(tui_capture)"; fi
    tui_keys M-i
    if wait_for "Activity" 5; then _pass "Alt+I opens the inspector"
    else _fail "Alt+I opens the inspector" "$(tui_capture)"; fi
    tui_keys Tab
    if wait_for "Credentials" 5; then _pass "Tab moves to the Session page"
    else _fail "Tab moves to the Session page" "$(tui_capture)"; fi
    tui_keys Escape
    if wait_absent "Credentials" 5; then _pass "Escape closes the inspector"
    else _fail "Escape closes the inspector" "$(tui_capture)"; fi
    assert_contains "$(tui_capture)" "draft one" "the draft survives the inspector"

    # ---- pasted newline and slash text never submit ----------------------
    tui_keys C-u
    tui_paste "pasted one"$'\r'"/clear"
    if wait_for "/clear" 5; then
      tui_settle
      s="$(tui_capture)"
      assert_contains "$s" "draft not sent" "pasted newline and /clear stay in the draft"
      assert_absent "$s" "you" "nothing was submitted to the transcript"
    else
      _fail "pasted text lands in the composer" "$(tui_capture)"
    fi
    tui_keys C-u

    # ---- submit, stream, scroll away, resize, back to live ---------------
    _tmux resize-window -t "$TUI_SESSION" -x 120 -y 40
    tui_type "stream please"; tui_keys Enter
    if wait_for "fake line 60" 15; then _pass "the response streams into the transcript"
    else _fail "the response streams into the transcript" "$(tui_capture)"; fi
    tui_keys PageUp
    if wait_for "scrolled up" 5; then _pass "scrolling away shows the way back to live"
    else _fail "scrolling away shows the way back to live" "$(tui_capture)"; fi
    local size
    for size in "80 24" "40 12"; do
      _tmux resize-window -t "$TUI_SESSION" -x "${size% *}" -y "${size#* }"
      tui_settle
      s="$(tui_capture)"
      assert_contains "$s" "scrolled up" "the scrolled view survives a resize to ${size/ /x}"
      assert_contains "$s" "fake line" "transcript text is still shown at ${size/ /x}"
      assert_contains "$s" "working" "the turn is still running at ${size/ /x}"
    done
    _tmux resize-window -t "$TUI_SESSION" -x 120 -y 40
    tui_keys M-l
    if wait_absent "scrolled up" 5; then _pass "Alt+L jumps back to live output"
    else _fail "Alt+L jumps back to live output" "$(tui_capture)"; fi

    # ---- cancel: the runtime stops the turn ------------------------------
    tui_keys Escape
    if wait_for "ready" 10; then _pass "Escape cancels the running turn"
    else _fail "Escape cancels the running turn" "$(tui_capture)"; fi
    local waited=0
    while pgrep -f "$scratch/bin/claude" >/dev/null 2>&1 && [ "$waited" -lt 10 ]; do
      sleep 0.5; waited=$((waited + 1))
    done
    if pgrep -f "$scratch/bin/claude" >/dev/null 2>&1; then
      _fail "the cancelled turn's claude process exits"
    else
      _pass "the cancelled turn's claude process exits"
    fi
    # The fake finishes 6s after answering; give it the time it would need.
    sleep 7
    if [ -e "$scratch/done" ]; then _fail "the cancelled turn did not run to completion"
    else _pass "the cancelled turn did not run to completion"; fi
    assert_contains "$(tui_capture)" "Cancelled." "the settled outcome is shown"

    # ---- quit and terminal restoration -----------------------------------
    tui_keys C-c; sleep 0.3; tui_keys C-c
    # Type the check only once the shell owns the pane again; keys sent
    # while the app is still exiting are read by it and lost.
    local waited=0
    while [ "$waited" -lt 50 ] && \
      [ "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" = "coven-code" ]; do
      sleep 0.1; waited=$((waited + 1))
    done
    tui_type 'stty -a | grep -q -- "-icanon" && echo "RAW_$((40 + 1))" || echo "COOKED_$((40 + 2))"'
    tui_keys Enter
    if wait_for "COOKED_42" 10; then _pass "quitting restores cooked terminal mode"
    else _fail "quitting restores cooked terminal mode" "$(tui_capture)"; fi
    assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{alternate_on}')" "0" \
      "quitting leaves the alternate screen"
    assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{cursor_flag}')" "1" \
      "quitting restores the cursor"
    assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{mouse_any_flag}')" "0" \
      "quitting turns mouse reporting off"
  fi
  tui_stop

  # ---- Classic: Escape stops the running turn as well ----------------------
  rm -f "$scratch/done"
  TUI_LAUNCH_PREFIX="${TUI_LAUNCH_PREFIX/COVEN_CODE_UI=next/COVEN_CODE_UI=classic}"
  TUI_BOOT_STRING="$saved_boot"
  if tui_start; then
    tui_type "stream please"; tui_keys Enter
    if wait_for "fake line 60" 15; then
      tui_keys Escape
      local tries=0
      while pgrep -f "$scratch/bin/claude" >/dev/null 2>&1 && [ "$tries" -lt 10 ]; do
        sleep 0.5; tries=$((tries + 1))
      done
      if pgrep -f "$scratch/bin/claude" >/dev/null 2>&1; then
        _fail "Classic: Escape stops the turn's claude process"
      else
        _pass "Classic: Escape stops the turn's claude process"
      fi
      sleep 7
      if [ -e "$scratch/done" ]; then _fail "Classic: the cancelled turn did not complete"
      else _pass "Classic: the cancelled turn did not complete"; fi
    else
      _fail "Classic: the response streams" "$(tui_capture)"
    fi
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  pkill -f "$scratch/bin/claude" 2>/dev/null || true
  rm -rf "$scratch"
}
