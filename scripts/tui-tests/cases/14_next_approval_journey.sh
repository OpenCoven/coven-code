#!/usr/bin/env bash
# shellcheck shell=bash
#
# Engine-local approval journey (#411): real Bash tool calls from a local fake
# of the Messages API go through the engine's real permission path, so each
# decision is checked three ways: on screen, by whether the command ran (its
# marker file), and by what the model was told. Everything stays on 127.0.0.1
# with a fake key. This is engine-local evidence only: protected, daemon-backed
# approvals are a different path and are not covered here.

register_case tc_next_approval_journey

_approval_requests() {
  if [ -f "$APPROVAL_LOG" ]; then wc -l <"$APPROVAL_LOG" | tr -d ' '; else echo 0; fi
}

# Wait until the fake has seen this many requests in total.
_wait_for_requests() {
  local waited=0
  while [ "$waited" -lt 100 ]; do
    [ "$(_approval_requests)" -ge "$1" ] && return 0
    sleep 0.1; waited=$((waited + 1))
  done
  return 1
}

# Submit a prompt and wait for its approval panel. The last word names the marker.
_ask_for_tool() {
  tui_type "please $1"; sleep 0.2; tui_keys Enter
  wait_for "Approve Bash" 15
}

# What the model was told about the last tool call: "ok", "error", or "none"
# when the last request carried no tool result (say, a wait that timed out).
_last_tool_result() {
  python3 -B -c 'import json, sys
lines = open(sys.argv[1], encoding="utf-8").read().splitlines()
content = json.loads(lines[-1])["last"].get("content") if lines else None
blocks = [b for b in content if isinstance(b, dict) and b.get("type") == "tool_result"] \
    if isinstance(content, list) else []
print("none" if not blocks else "error" if blocks[-1].get("is_error") else "ok")' "$APPROVAL_LOG"
}

# Let the panel arm: keys are ignored until 400 ms pass with none pressed.
_let_panel_arm() { sleep 0.7; }

tc_next_approval_journey() {
  describe "Next approves and denies real engine-local tool calls"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/acme-next" "$scratch/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  APPROVAL_LOG="$scratch/requests.log"

  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$scratch/port" "$APPROVAL_LOG" "$scratch/markers" &
  local fake_pid=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -s "$scratch/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  if [ ! -s "$scratch/port" ]; then
    _fail "the fake API starts"
    kill "$fake_pid" 2>/dev/null
    rm -rf "$scratch"
    return 0
  fi

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-next") && env"
  local var
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$(cat "$scratch/port")")"
  TUI_BOOT_STRING="preview"

  if tui_start; then
    local s requests

    # ---- Enter on a new request denies ------------------------------------
    if _ask_for_tool enter-marker; then
      _pass "a tool call opens an approval panel"
      s="$(tui_capture)"
      assert_contains "$s" "› [n] No, deny" "the highlighted choice is deny"
      assert_contains "$s" "no daemon protection applies" "the panel names the real enforcer"
      assert_contains "$s" "awaiting permission" "the status says a decision is pending"
      if [ -e "$scratch/markers/enter-marker" ]; then _fail "nothing runs before a decision"
      else _pass "nothing runs before a decision"; fi
      _let_panel_arm
      tui_keys Enter
      if _wait_for_requests 2 && wait_for "TOOL_RESULT_ERROR" 10; then
        _pass "Enter refuses and the model hears an error"
      else
        _fail "Enter refuses and the model hears an error" "$(tui_capture)"
      fi
      if [ -e "$scratch/markers/enter-marker" ]; then _fail "a denied command does not run"
      else _pass "a denied command does not run"; fi
      assert_eq "$(_last_tool_result)" "error" "the tool result is an error"
      assert_contains "$(tui_capture)" "denied" "the tool row says denied"
    else
      _fail "a tool call opens an approval panel" "$(tui_capture)"
    fi

    # ---- keys typed around arrival do not approve -------------------------
    # Rushed Enters and a burst of y must not reach the panel as a decision.
    requests="$(_approval_requests)"
    tui_type "please rushed-marker"; sleep 0.2
    tui_keys Enter; tui_keys Enter; tui_keys Enter
    local burst=0 open_keys=0
    while [ "$burst" -lt 8 ]; do
      if grep -q "Approve Bash" <<<"$(tui_capture)"; then open_keys=$((open_keys + 1)); fi
      tui_keys y; sleep 0.1; burst=$((burst + 1))
    done
    if [ "$open_keys" -gt 0 ]; then _pass "part of the burst reached an open panel"
    else _fail "part of the burst reached an open panel" "$(tui_capture)"; fi
    if [ -e "$scratch/markers/rushed-marker" ]; then
      _fail "keys typed as the panel arrives do not approve"
    else
      _pass "keys typed as the panel arrives do not approve"
    fi
    # Settle whatever state that left: a still-open panel, or an earlier deny.
    _let_panel_arm
    if grep -q "Approve Bash" <<<"$(tui_capture)"; then tui_keys Escape; fi
    _wait_for_requests $((requests + 2)) || true
    wait_for "ready" 10 || true
    tui_keys C-u
    if [ -e "$scratch/markers/rushed-marker" ]; then
      _fail "the rushed command never ran"
    else
      _pass "the rushed command never ran"
    fi

    # ---- allow once runs once and is not remembered -----------------------
    requests="$(_approval_requests)"
    if _ask_for_tool once-marker; then
      _let_panel_arm
      tui_keys y
      if _wait_for_requests $((requests + 2)) && wait_for "TOOL_RESULT_OK" 10; then
        _pass "allow once runs the call and the model hears success"
      else
        _fail "allow once runs the call and the model hears success" "$(tui_capture)"
      fi
      if [ -e "$scratch/markers/once-marker" ]; then _pass "the allowed command ran"
      else _fail "the allowed command ran"; fi
      assert_eq "$(_last_tool_result)" "ok" "the tool result is not an error"
    else
      _fail "allow once runs the call and the model hears success" "$(tui_capture)"
    fi

    wait_for "ready" 10 || true
    requests="$(_approval_requests)"
    if _ask_for_tool second-marker; then
      _pass "allow once does not cover the next call"
      if [ -e "$scratch/markers/second-marker" ]; then _fail "the next call waits for a decision"
      else _pass "the next call waits for a decision"; fi

      # ---- Escape refuses ---------------------------------------------
      tui_keys Escape
      if _wait_for_requests $((requests + 2)); then _pass "Escape refuses the call"
      else _fail "Escape refuses the call" "$(tui_capture)"; fi
      if [ -e "$scratch/markers/second-marker" ]; then _fail "an escaped command does not run"
      else _pass "an escaped command does not run"; fi
      assert_eq "$(_last_tool_result)" "error" "Escape tells the model the call failed"
    else
      _fail "allow once does not cover the next call" "$(tui_capture)"
    fi

    # ---- allow for the session covers this command, and stays in the session
    wait_for "ready" 10 || true
    requests="$(_approval_requests)"
    local settings_before
    settings_before="$(cat "$scratch/home/.coven-code/settings.json")"
    if _ask_for_tool session-marker; then
      _let_panel_arm
      tui_keys Y
      _wait_for_requests $((requests + 2)) || true
      if [ -e "$scratch/markers/session-marker" ]; then _pass "allow for the session runs the call"
      else _fail "allow for the session runs the call" "$(tui_capture)"; fi
      assert_eq "$(_last_tool_result)" "ok" "allow for the session tells the model it succeeded"
      wait_for "ready" 10 || true

      rm -f "$scratch/markers/session-marker"
      requests="$(_approval_requests)"
      tui_type "please session-marker"; sleep 0.2; tui_keys Enter
      if _wait_for_requests $((requests + 2)); then _pass "the same command needs no new decision"
      else _fail "the same command needs no new decision" "$(tui_capture)"; fi
      if [ -e "$scratch/markers/session-marker" ]; then _pass "the same command ran again"
      else _fail "the same command ran again" "$(tui_capture)"; fi
      assert_eq "$(_last_tool_result)" "ok" "the model hears the repeated call succeeded"
      wait_for "ready" 10 || true

      requests="$(_approval_requests)"
      if _ask_for_tool other-marker; then _pass "a different command still asks"
      else _fail "a different command still asks" "$(tui_capture)"; fi
      if [ -e "$scratch/markers/other-marker" ]; then _fail "a different command waits for a decision"
      else _pass "a different command waits for a decision"; fi
      tui_keys Escape
      _wait_for_requests $((requests + 2)) || true
      assert_eq "$(cat "$scratch/home/.coven-code/settings.json")" "$settings_before" \
        "allow for the session saves nothing to settings"
    else
      _fail "allow for the session runs the call" "$(tui_capture)"
    fi
  fi
  tui_stop

  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  unset APPROVAL_LOG
  rm -rf "$scratch"
}
