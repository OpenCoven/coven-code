#!/usr/bin/env bash
# shellcheck shell=bash
#
# Engine-local approval edges (#411), through the same local fake of the Messages
# API as 14_next_approval_journey.sh: a file changing during its approval, a second
# decision key, typing a session command at an open panel, a denied path, and a
# command that prints and then fails. Engine-local evidence only.

register_case tc_next_approval_edges

_edge_ask() { # prompt, panel title
  tui_type "$1"; sleep 0.3; tui_keys Enter
  wait_for "$2" 15
}

_edge_requests() {
  if [ -f "$EDGE_LOG" ]; then wc -l <"$EDGE_LOG" | tr -d ' '; else echo 0; fi
}

# What the model was told in the request with this number, waiting for it to
# arrive. Text on screen will not do: an earlier step's result can still be
# showing. Neither will the last request: a later one can already be in.
_edge_told_after() { # request number
  local waited=0
  while [ "$waited" -lt 150 ] && [ "$(_edge_requests)" -lt "$1" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  sed -n "$1p" "$EDGE_LOG" 2>/dev/null
}

_edge_clear() { tui_keys C-u; sleep 0.2; }

# Shown by the session browser. See 09_next_sessions.sh.
EDGE_BROWSER="Last Updated"

tc_next_approval_edges() {
  describe "Next approval edges with real engine-local tools"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  local project="$scratch/acme-next"
  mkdir -p "$scratch/home/.coven-code" "$project/forbidden" "$scratch/markers"
  cat >"$scratch/home/.coven-code/settings.json" <<JSON
{"hasCompletedOnboarding": true,
 "permissionRules": [{"tool_name": "Write", "path_pattern": "*/forbidden/*", "action": "Deny"}]}
JSON
  EDGE_LOG="$scratch/requests.log"

  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" "$scratch/port" "$EDGE_LOG" "$scratch/markers" &
  local fake_pid=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -s "$scratch/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  local saved_width="$TUI_WIDTH" saved_height="$TUI_HEIGHT"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$project") && env"
  local var
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$(cat "$scratch/port" 2>/dev/null)")"
  TUI_BOOT_STRING="preview"
  # Tall enough that the long file paths in a panel do not push out its options.
  TUI_WIDTH=160 TUI_HEIGHT=50

  if [ -s "$scratch/port" ] && tui_start; then
    _next_approval_edges_journey "$project" "$scratch"
  fi
  tui_stop
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  TUI_WIDTH="$saved_width" TUI_HEIGHT="$saved_height"
  unset EDGE_LOG
  rm -rf "$scratch"
}

_next_approval_edges_journey() {
  local project="$1" scratch="$2" before told

  # ---- a Write approved after its file changed does not overwrite the change
  printf 'as reviewed\n' >"$project/report.txt"
  before="$(_edge_requests)"
  if _edge_ask "tool: Write {\"file_path\":\"$project/report.txt\",\"content\":\"from the model\\n\"}" "Approve Write"; then
    printf 'changed by someone\n' >"$project/report.txt"
    sleep 0.5; tui_keys y
    told="$(_edge_told_after $((before + 2)))"
    assert_contains "$told" "changed while waiting for approval" "the model is told a Write's file changed"
    assert_eq "$(cat "$project/report.txt")" "changed by someone" "a Write does not overwrite the other writer's change"
  else
    _fail "Write asks for approval" "$(tui_capture)"
  fi
  wait_absent "working" 15 >/dev/null
  _edge_clear

  # ---- an Edit approved after its file changed does not overwrite the change
  printf 'old line\n' >"$project/notes.txt"
  before="$(_edge_requests)"
  if _edge_ask "tool: Edit {\"file_path\":\"$project/notes.txt\",\"old_string\":\"old line\",\"new_string\":\"new line\"}" "Approve Edit"; then
    printf 'changed by someone\n' >"$project/notes.txt"
    sleep 0.7; tui_keys y
    told="$(_edge_told_after $((before + 2)))"
    assert_contains "$told" "changed while waiting for approval" "an Edit of a changed file fails after approval"
    assert_contains "$told" '"is_error": true' "the model is told the Edit failed"
    assert_eq "$(cat "$project/notes.txt")" "changed by someone" "the other writer's change survives"
  else
    _fail "Edit asks for approval" "$(tui_capture)"
  fi
  wait_for "ready" 10 >/dev/null || true

  # ---- a second allow key after the decision does not run the call again or
  # start another request. The fake sends one call per response, so this cannot
  # show a second key reaching the next queued panel: that needs two calls in
  # one response.
  before="$(_edge_requests)"
  if _edge_ask "run: echo once >> $project/count.txt" "Approve Bash"; then
    sleep 0.7; tui_keys y; tui_keys y
    told="$(_edge_told_after $((before + 2)))"
    assert_absent "$told" '"is_error": true' "a doubled allow key still runs the call"
    wait_for "ready" 10 >/dev/null || true
    assert_eq "$(wc -l <"$project/count.txt" | tr -d ' ')" "1" "a doubled allow key runs the command once"
    assert_eq "$(_edge_requests)" "$((before + 2))" "a doubled allow key starts no other request"
    _edge_clear
  else
    _fail "Bash asks for approval" "$(tui_capture)"
  fi

  # ---- a session command typed at an open panel cannot switch sessions. Its n
  # denies the call; the text after that reaches the composer and is a new prompt,
  # whose own call asks again.
  before="$(_edge_requests)"
  if _edge_ask "run: touch $scratch/markers/not-switched" "Approve Bash"; then
    sleep 0.7
    tui_type "/session resume"; tui_keys Enter
    told="$(_edge_told_after $((before + 2)))"
    assert_contains "$told" '"is_error": true' "the n in a session command denies the call"
    assert_eq "$([ -e "$scratch/markers/not-switched" ] && echo ran || echo not-run)" "not-run" \
      "the denied call did not run"
    assert_absent "$(tui_capture)" "$EDGE_BROWSER" "no session browser opened"
    if wait_for "Approve Bash" 8; then
      _pass "the leftover text asks again instead of running"
      tui_keys Escape
    else
      _fail "the leftover text asks again instead of running" "$(tui_capture)"
    fi
    _edge_told_after $((before + 4)) >/dev/null
    wait_for "ready" 10 >/dev/null || true
    assert_eq "$([ -e "$scratch/markers/resume" ] && echo ran || echo not-run)" "not-run" \
      "the leftover text's call did not run"
    # The browser check above can fail: the same keys with no panel open it.
    tui_type "/session resume"; tui_keys Enter
    if wait_for "$EDGE_BROWSER" 5; then _pass "the same keys with no panel open the browser"
    else _fail "the same keys with no panel open the browser" "$(tui_capture)"; fi
    tui_keys Escape; sleep 0.3; _edge_clear
  else
    _fail "Bash asks for approval" "$(tui_capture)"
  fi

  # ---- a denied path never reaches a panel or the disk
  before="$(_edge_requests)"
  tui_type "tool: Write {\"file_path\":\"$project/forbidden/x.txt\",\"content\":\"no\"}"; sleep 0.3; tui_keys Enter
  told="$(_edge_told_after $((before + 2)))"
  assert_contains "$told" '"is_error": true' "a denied path is refused without asking"
  assert_absent "$(tui_capture)" "Approve Write" "no panel is left open for a denied path"
  assert_eq "$([ -e "$project/forbidden/x.txt" ] && echo written || echo absent)" "absent" \
    "a denied path is not written"
  wait_for "ready" 10 >/dev/null || true

  # ---- output before a failure reaches the model, and the failure is a failure
  before="$(_edge_requests)"
  if _edge_ask "run: echo partial-out; exit 3" "Approve Bash"; then
    sleep 0.7; tui_keys y
    told="$(_edge_told_after $((before + 2)))"
    assert_contains "$told" '"is_error": true' "a failing command is reported as failed"
    assert_contains "$told" "partial-out" "its output before the failure reaches the model"
  else
    _fail "Bash asks for approval" "$(tui_capture)"
  fi
}
