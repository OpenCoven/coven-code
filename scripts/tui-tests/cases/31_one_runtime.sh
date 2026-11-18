#!/usr/bin/env bash
# shellcheck shell=bash
#
# One runtime per renderer (#408). Starting Classic (COVEN_CODE_UI unset) or
# Next (COVEN_CODE_UI=next) must keep one session, one provider client and one
# store for the whole run. A local fake of the Messages API drives two real
# turns, an approved tool call and then a text answer, with a full redraw (a
# resize there and back) between them. The checks read only what can be seen
# from outside the process:
# - session: one session file, with the same id after the first turn, after
#   the redraw, after the second turn and after the exit; the session index
#   holds that one session;
# - provider: three streamed requests, never two at once; each carries the
#   one before it and its answer; the second turn's request arrives on a TCP
#   connection that the first turn's requests used, which only the same
#   client's connection pool can do;
# - store: the saved session holds both turns as the model saw them, with the
#   last answer, and every indexed message belongs to that one session.
# It does not prove that nothing else is built: a second session, provider
# client, scheduler or store that is constructed but never used to send or
# save leaves no trace here. Only the direct API-key Claude transport runs.

register_case tc_one_runtime

ORT_TOOL_PROMPT="run: echo one-runtime-first"
ORT_TEXT_PROMPT="flood: 2"

# Start the TUI for one renderer against the fake API, with its own home.
_ort_launch() { # renderer, home, port, project
  local var ui=""
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$4") && env"
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  if [ "$1" = next ]; then ui="COVEN_CODE_UI=next"; TUI_BOOT_STRING="preview"; ORT_BUSY="working"
  else TUI_BOOT_STRING="Coven v"; ORT_BUSY="… · "; fi
  # shellcheck disable=SC2086 # an empty selector adds nothing
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$2" "COVEN_HOME=$2/.coven" $ui \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$3")"
  tui_start
}

_ort_lines() { # file
  if [ -f "$1" ]; then wc -l <"$1" | tr -d ' '; else echo 0; fi
}

# Wait until the fake has answered this many streamed requests.
_ort_wait_for_requests() { # messages log, count
  local waited=0
  while [ "$waited" -lt 150 ]; do
    [ "$(_ort_lines "$1")" -ge "$2" ] && return 0
    sleep 0.1; waited=$((waited + 1))
  done
  return 1
}

# Wait until the turn no longer shows as running.
_ort_wait_idle() {
  local waited=0
  while [ "$waited" -lt 50 ]; do
    grep -qF -- "$ORT_BUSY" <<<"$(tui_capture)" || return 0
    sleep 0.2; waited=$((waited + 1))
  done
  return 1
}

# Wait until a saved session names `text`; a turn is saved when it ends.
_ort_wait_saved() { # home, text
  local waited=0
  while [ "$waited" -lt 100 ]; do
    find "$1" -type f -path '*/sessions/*.json' -exec grep -qF -- "$2" {} + 2>/dev/null && return 0
    sleep 0.1; waited=$((waited + 1))
  done
  return 1
}

# Report each check on a line of its own: "ok", "FAIL" or "id", a tab, the
# check, a tab and what was seen. `first` runs after the first turn and
# reports the session id; `final` runs after the exit and compares with it.
_ort_check() { # first|final, home, messages log, connections log, first id
  python3 -B -c '
import glob, json, os, pathlib, sqlite3, sys

mode, home, messages_log, connections_log, tool_prompt, text_prompt, first_id = sys.argv[1:8]
config = os.path.join(home, ".coven-code")

def report(label, ok, seen=""):
    seen = " ".join(str(seen).split())
    print(("ok" if ok else "FAIL") + "\t" + label + "\t" + seen[:2000])

def session_ids():
    files = sorted(glob.glob(os.path.join(config, "sessions", "*.json")))
    return files, [os.path.basename(f)[: -len(".json")] for f in files]

def blocks(message):
    content = message.get("content")
    if isinstance(content, str):
        return [{"type": "text", "text": content}]
    return [b for b in content or [] if isinstance(b, dict)]

def signature(messages):
    # What both the wire and the saved file agree on: roles, text, and which
    # call each tool result answers.
    out = []
    for message in messages:
        parts = []
        for b in blocks(message):
            kind = b.get("type")
            if kind == "text":
                parts.append(("text", b.get("text", "").strip()))
            elif kind == "tool_use":
                parts.append(("tool_use", b.get("id"), b.get("name")))
            elif kind == "tool_result":
                parts.append(("tool_result", b.get("tool_use_id")))
            else:
                parts.append((kind,))
        out.append((message.get("role"), parts))
    return out

def without_cache_control(value):
    if isinstance(value, dict):
        return {k: without_cache_control(v) for k, v in value.items() if k != "cache_control"}
    if isinstance(value, list):
        return [without_cache_control(v) for v in value]
    return value

def read_lines(path):
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]

files, ids = session_ids()
if mode == "first":
    report("one session file after the first turn", len(ids) == 1, ids)
    if len(ids) == 1:
        with open(files[0], encoding="utf-8") as f:
            saved_id = json.load(f).get("id")
        report("the session file is named for the id it holds", saved_id == ids[0], saved_id)
        print("id\t" + ids[0] + "\t")
    sys.exit(0)

report("still one session file, with the id of the first turn, after the redraw, the second turn and the exit",
       ids == [first_id], "files: %s; first turn: %s" % (ids, first_id))

db_path = os.path.join(config, "sessions.db")
if os.path.exists(db_path):
    db = sqlite3.connect(pathlib.Path(db_path).as_uri() + "?mode=ro", uri=True)
    indexed = [row[0] for row in db.execute("SELECT id FROM sessions")]
    owners = sorted({row[0] for row in db.execute("SELECT session_id FROM messages")})
    texts = [row[0] for row in db.execute("SELECT content FROM messages")]
    db.close()
else:
    indexed, owners, texts = None, None, []
report("the session index holds one session, with that id", indexed == [first_id], indexed)
# The index keys a message by its uuid, and a user message has none, so
# every one shares the id "unknown" and only the first is kept. The answers
# carry their own, so they show both turns reached the index.
answers = [any(text.strip() == want for text in texts)
           for want in ("TOOL_RESULT_OK", "flood line\nflood line")]
report("every indexed message belongs to that session, the answer of each turn included",
       owners == [first_id] and all(answers),
       "owners: %s; answers indexed: %s" % (owners, answers))

requests = [line["messages"] for line in read_lines(messages_log)]
streams = sorted((c for c in read_lines(connections_log) if c.get("stream")),
                 key=lambda c: c["started"])
report("the model saw three streamed requests: the prompt, the tool result and the second prompt",
       len(requests) == 3 and len(streams) == 3,
       "%d logged with messages, %d streamed" % (len(requests), len(streams)))
overlaps = [(a["started"], a["ended"], b["started"])
            for a, b in zip(streams, streams[1:]) if b["started"] < a["ended"]]
report("no two streamed requests were open at once", len(streams) > 1 and not overlaps, overlaps)

carried = len(requests) == 3
for before, after in zip(requests, requests[1:]):
    before, after = without_cache_control(before), without_cache_control(after)
    if len(after) != len(before) + 2 or after[: len(before)] != before:
        carried = False
if requests:
    carried = carried and signature(requests[0]) == [("user", [("text", tool_prompt)])] \
        and signature(requests[-1])[-1] == ("user", [("text", text_prompt)])
report("each request carries the one before it, its answer and the next user message, from the first prompt on",
       carried, [signature(r) for r in requests])

if len(streams) == 3:
    first_turn = {streams[0]["connection"], streams[1]["connection"]}
    report("the request of the second turn came on a connection the first turn used",
           streams[2]["connection"] in first_turn,
           "first turn: %s; second turn: %s" % (sorted(first_turn), streams[2]["connection"]))
else:
    report("the request of the second turn came on a connection the first turn used", False, streams)

saved = []
if len(files) == 1:
    with open(files[0], encoding="utf-8") as f:
        saved = json.load(f).get("messages") or []
answer = ("assistant", [("text", "flood line\nflood line")])
expected = signature(requests[-1]) + [answer] if requests else None
report("the saved session is the conversation the model saw, and its last answer",
       signature(saved) == expected, "saved: %s; expected: %s" % (signature(saved), expected))
' "$1" "$2" "$3" "$4" "$ORT_TOOL_PROMPT" "$ORT_TEXT_PROMPT" "${5:-}"
}

# Turn the checker's lines into results. Sets ORT_ID from an "id" line.
_ort_report() { # renderer, then the checker's arguments
  local r="$1" verdict label seen
  shift
  while IFS=$'\t' read -r verdict label seen; do
    case "$verdict" in
      ok) _pass "$r: $label" ;;
      id) ORT_ID="$label" ;;
      *) _fail "$r: ${label:-the checker runs}" "${seen:-$verdict}" ;;
    esac
  done < <(_ort_check "$@" 2>&1)
}

tc_one_runtime() {
  describe "One session, provider client and store per renderer"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" r
  for r in classic next; do
    _ort_journey "$r" "$scratch/$r"
  done
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

# Two turns with a redraw between them, with a fake of its own so its logs
# hold this renderer's requests only.
_ort_journey() { # renderer, dir
  local r="$1" dir="$2" waited=0 fake_pid
  mkdir -p "$dir/home/.coven-code" "$dir/project" "$dir/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$dir/home/.coven-code/settings.json"
  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" "$dir/port" "$dir/requests.log" \
    "$dir/markers" "$dir/messages.log" "$dir/connections.log" 2>/dev/null &
  fake_pid=$!
  while [ "$waited" -lt 50 ] && [ ! -s "$dir/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  if [ ! -s "$dir/port" ]; then
    _fail "$r: the fake API starts"
    kill "$fake_pid" 2>/dev/null; wait "$fake_pid" 2>/dev/null
    return 0
  fi

  ORT_ID=""
  if _ort_launch "$r" "$dir/home" "$(cat "$dir/port")" "$dir/project" && _ort_turns "$r" "$dir"; then
    _ort_report "$r" final "$dir/home" "$dir/messages.log" "$dir/connections.log" "$ORT_ID"
  fi
  tui_stop
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
}

# Returns nonzero when the journey cannot reach the final checks.
_ort_turns() { # renderer, dir
  local r="$1" dir="$2" log="$2/messages.log" running size
  running="$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')"

  tui_type "$ORT_TOOL_PROMPT"; sleep 0.3; tui_keys Enter
  if ! wait_for "allow once" 15; then
    _fail "$r: the Bash call asks for approval" "$(tui_capture)"; return 1
  fi
  sleep 0.7; tui_keys y
  if ! wait_for "TOOL_RESULT_OK" 15 || ! _ort_wait_idle; then
    _fail "$r: the first turn ends with the model's answer" "$(tui_capture)"; return 1
  fi
  if ! _ort_wait_saved "$dir/home" "TOOL_RESULT_OK"; then
    _fail "$r: the first turn is saved" "$(find "$dir/home" -type f)"; return 1
  fi
  _pass "$r: the first turn ends and is saved"
  _ort_report "$r" first "$dir/home" "$log" "$dir/connections.log"
  if [ -z "$ORT_ID" ]; then return 1; fi

  # A full redraw: tmux holds a resize for 250 ms, so each size is kept past
  # that. 100x30 differs from both suite sizes.
  for size in "100 30" "$TUI_WIDTH $TUI_HEIGHT"; do
    _tmux resize-window -t "$TUI_SESSION" -x "${size% *}" -y "${size#* }"
    sleep 0.6
  done
  if wait_for "TOOL_RESULT_OK" 20 && \
    [ "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" = "$running" ]; then
    _pass "$r: the redraw keeps the process and draws the first answer again"
  else
    _fail "$r: the redraw keeps the process and draws the first answer again" "$(tui_capture)"
    return 1
  fi

  tui_type "$ORT_TEXT_PROMPT"; sleep 0.3; tui_keys Enter
  if ! _ort_wait_for_requests "$log" 3 || ! wait_for "flood line" 15 || ! _ort_wait_idle; then
    _fail "$r: the second turn ends with the model's answer" \
      "$(_ort_lines "$log") requests; $(tui_capture)"
    return 1
  fi
  if ! _ort_wait_saved "$dir/home" "$ORT_TEXT_PROMPT"; then
    _fail "$r: the second turn is saved" "$(find "$dir/home" -type f)"; return 1
  fi
  _pass "$r: the second turn ends and is saved"
  tui_quit_and_check "$r: Ctrl+C twice after the turns" C-c C-c
}
