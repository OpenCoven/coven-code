#!/usr/bin/env bash
# shellcheck shell=bash
#
# Tool results stay in the conversation after a tool turn, in Classic and in
# Next. A local fake of the Messages API drives a real turn through the real
# client and query loop: the model asks for a Bash call, the user approves it,
# and the model answers its result. The next prompt's request must still carry
# that call and the tool result answering it; a call with no result is a
# request the Messages API rejects. The first key pressed after the turn is
# where the loop used to take the App's shorter transcript (it shows results
# on tool cards, without the messages carrying them) for a rewind, so the
# journey presses keys one at a time, as typing does, before the next prompt.
# Classic runs with COVEN_CODE_UI unset, Next with COVEN_CODE_UI=next.

register_case tc_tool_results_kept

_trk_requests() { # log
  if [ -f "$1" ]; then wc -l <"$1" | tr -d ' '; else echo 0; fi
}

# Wait until the fake has seen this many streamed requests in total.
_trk_wait_for_requests() { # log, count
  local waited=0
  while [ "$waited" -lt 150 ]; do
    [ "$(_trk_requests "$1")" -ge "$2" ] && return 0
    sleep 0.1; waited=$((waited + 1))
  done
  return 1
}

# "ok" when the last request's history holds a tool call and every call is
# answered by a tool result with its id in the user message right after it,
# as the Messages API requires; otherwise the history's shape.
_trk_last_request_shape() { # messages log
  python3 -B -c 'import json, sys
lines = open(sys.argv[1], encoding="utf-8").read().splitlines()
messages = json.loads(lines[-1])["messages"]
def blocks(message):
    content = message.get("content")
    return [b for b in content if isinstance(b, dict)] if isinstance(content, list) else []
def ids(message, kind, key):
    return [b.get(key) for b in blocks(message) if b.get("type") == kind]
answered, shape = False, []
for message in messages:
    calls = ids(message, "tool_use", "id")
    results = ids(message, "tool_result", "tool_use_id")
    kinds = ["tool_use " + str(c) for c in calls] + ["tool_result " + str(r) for r in results]
    shape.append(message.get("role", "?") + (" (" + ", ".join(kinds) + ")" if kinds else ""))
for i, message in enumerate(messages):
    calls = ids(message, "tool_use", "id")
    if not calls:
        continue
    following = messages[i + 1] if i + 1 < len(messages) else {}
    answered = following.get("role") == "user" and \
        set(calls) <= set(ids(following, "tool_result", "tool_use_id"))
    if not answered:
        break
print("ok" if answered else "; ".join(shape))' "$1"
}

# Wait until the turn no longer shows as running.
_trk_wait_idle() {
  local waited=0
  while [ "$waited" -lt 50 ]; do
    grep -qF -- "$TRK_BUSY" <<<"$(tui_capture)" || return 0
    sleep 0.2; waited=$((waited + 1))
  done
  return 1
}

# Start the TUI for one renderer against the fake API, with its own home.
_trk_launch() { # renderer, home, port, project
  local var ui=""
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$4") && env"
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  if [ "$1" = next ]; then ui="COVEN_CODE_UI=next"; TUI_BOOT_STRING="preview"; TRK_BUSY="working"
  else TUI_BOOT_STRING="Coven v"; TRK_BUSY="… · "; fi
  # shellcheck disable=SC2086 # an empty selector adds nothing
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$2" "COVEN_HOME=$2/.coven" $ui \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$3")"
  tui_start
}

tc_tool_results_kept() {
  describe "The next prompt after a tool turn still carries the tool result"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" r
  for r in classic next; do
    _trk_journey "$r" "$scratch/$r"
  done
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

# One approved tool turn, then a follow-up prompt, with a fake of its own so
# its logs hold this renderer's requests only.
_trk_journey() { # renderer, dir
  local r="$1" dir="$2" waited=0 fake_pid
  mkdir -p "$dir/home/.coven-code" "$dir/project" "$dir/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$dir/home/.coven-code/settings.json"
  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$dir/port" "$dir/requests.log" "$dir/markers" "$dir/messages.log" 2>/dev/null &
  fake_pid=$!
  while [ "$waited" -lt 50 ] && [ ! -s "$dir/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  if [ ! -s "$dir/port" ]; then
    _fail "$r: the fake API starts"
    kill "$fake_pid" 2>/dev/null; wait "$fake_pid" 2>/dev/null
    return 0
  fi

  _trk_launch "$r" "$dir/home" "$(cat "$dir/port")" "$dir/project" && _trk_turns "$r" "$dir"
  tui_stop
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
}

_trk_turns() { # renderer, dir
  # The fake writes this file last for each request.
  local r="$1" log="$2/messages.log" shape
  tui_type "run: echo first-tool"; sleep 0.3; tui_keys Enter
  if ! wait_for "allow once" 15; then
    _fail "$r: the Bash call asks for approval" "$(tui_capture)"; return 0
  fi
  sleep 0.7; tui_keys y
  if ! wait_for "TOOL_RESULT_OK" 15; then
    _fail "$r: the model answers the approved call's result" "$(tui_capture)"; return 0
  fi
  _pass "$r: the model answers the approved call's result"
  if ! _trk_wait_idle; then
    _fail "$r: the turn ends" "$(tui_capture)"; return 0
  fi
  # Press keys one at a time, as typing does, over a couple of seconds, so
  # some land after the loop has joined the finished turn.
  local _
  for _ in 1 2 3; do
    tui_keys x; sleep 0.3; tui_keys BSpace; sleep 0.4
  done
  tui_type "run: echo second"; sleep 0.3; tui_keys Enter
  if ! _trk_wait_for_requests "$log" 3; then
    _fail "$r: the follow-up prompt reaches the model" \
      "$(_trk_requests "$log") requests; $(tui_capture)"
    return 0
  fi
  shape="$(_trk_last_request_shape "$log")"
  if [ "$shape" = ok ]; then
    _pass "$r: the follow-up request carries the call and its tool result"
  else
    _fail "$r: the follow-up request carries the call and its tool result" "$shape"
  fi
}
