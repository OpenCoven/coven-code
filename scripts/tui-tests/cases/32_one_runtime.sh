#!/usr/bin/env bash
# shellcheck shell=bash
#
# One runtime per renderer (#408). Starting Classic (COVEN_CODE_UI unset) or
# Next (COVEN_CODE_UI=next) must keep one session, one provider client and one
# store for the whole run. A local fake of the Messages API drives two real
# turns, an approved tool call and then a text answer, with a redraw between
# them: the window grows by 20 columns and 5 rows, the app draws a frame that
# wide, and the window goes back. The checks read only what can be seen from
# outside the process:
# - session: one session file after the first turn and, after the second turn
#   and the exit, still that one file with that id; under the home and the
#   project, the only session file (sessions/*.json) and session index
#   (sessions.db) are that file and that index, which holds that one session;
# - provider: three streamed requests; each carries the one before it, the
#   fake's reply to it and the next user message; the request of the second
#   turn arrives on a TCP connection the first turn used, so it came from the
#   same HTTP connection pool (one reqwest client, or a clone of it); the
#   tool-result request inside the first turn is not checked on its own;
# - store: the saved session holds both turns (roles, text and tool ids) as
#   the model saw them, with the last answer, and every indexed message
#   belongs to that one session.
# What it does not show:
# - that nothing else is built. A session, client, scheduler or store that is
#   built but never sends or saves leaves no trace, and clients that share
#   one pool cannot be told apart. The process builds other clients, the
#   same way in either renderer: main() registers an Anthropic provider with
#   a pool of its own (used only without an API key or for another
#   provider); each goal turn wraps the client in a new AnthropicClient on
#   the same pool; each Agent tool sub-agent builds its own client; memory
#   extraction builds its own client with the default API base, ignoring
#   ANTHROPIC_BASE_URL (reached at 20 messages; this run has at most 6); and
#   /refresh and an in-session login replace the client and the registry by
#   design. None of those paths runs here;
# - one store object: the session index is opened again at every save, so
#   "one store" here means one file on disk;
# - the scheduler and the memory authority, which nothing here observes;
# - a switch of renderer: the selector is read once at start, so each
#   renderer runs in a process and home of its own, and relaunching the other
#   renderer on the same session is not covered; nor are /resume and /clear,
#   which change the session within a run;
# - any transport but the direct API-key Claude one.

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
    # find exits 0 when it finds no file at all, so test what grep names.
    [ -n "$(find "$1" -type f -path '*/sessions/*.json' -exec grep -lF -- "$2" {} + 2>/dev/null)" ] \
      && return 0
    sleep 0.1; waited=$((waited + 1))
  done
  return 1
}

# Report each check on a line of its own: "ok", "FAIL" or "id", a tab, the
# check, a tab and what was seen, and "end" last. `first` runs after the
# first turn and reports the session id; `final` runs after the exit and
# compares with it.
_ort_check() { # first|final, home, project, messages log, connections log, first id
  python3 -B -c '
import glob, json, os, pathlib, sqlite3, sys, time

mode, home, project, messages_log, connections_log, tool_prompt, text_prompt, first_id = sys.argv[1:9]
# With COVEN_HOME set, the app moves its home to COVEN_HOME/code and leaves
# .coven-code as a link to it; the walk below does not follow links.
config = os.path.realpath(os.path.join(home, ".coven-code"))

def report(label, ok, seen=""):
    seen = " ".join(str(seen).split())
    print(("ok" if ok else "FAIL") + "\t" + label + "\t" + seen[:2000])

def session_ids():
    files = sorted(glob.glob(os.path.join(config, "sessions", "*.json")))
    return files, [os.path.basename(f)[: -len(".json")] for f in files]

def load(path):
    # The first save writes the file in place; read it again if it is short.
    for _ in range(10):
        try:
            with open(path, encoding="utf-8") as f:
                return json.load(f)
        except ValueError:
            time.sleep(0.2)
    with open(path, encoding="utf-8") as f:
        return json.load(f)

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
        saved_id = load(files[0]).get("id")
        report("the session file is named for the id it holds", saved_id == ids[0], saved_id)
        print("id\t" + ids[0] + "\t")
    print("end\t\t")
    sys.exit(0)

saved_id = load(files[0]).get("id") if len(files) == 1 else None
report("after the second turn and the exit, still one session file, with the id of the first turn",
       ids == [first_id] and saved_id == first_id,
       "files: %s; id inside: %s; first turn: %s" % (ids, saved_id, first_id))
elsewhere = []
for root in (home, project):
    for parent, _, names in os.walk(root):
        for name in names:
            path = os.path.join(parent, name)
            if (name.endswith(".json") and os.path.basename(parent) == "sessions") \
                    or name == "sessions.db":
                elsewhere.append(os.path.realpath(path))
expected_files = sorted([os.path.join(config, "sessions", first_id + ".json"),
                         os.path.join(config, "sessions.db")])
report("under the home and the project, the only session file and session index are those",
       sorted(elsewhere) == expected_files,
       [os.path.relpath(path, os.path.realpath(os.path.dirname(home))) for path in sorted(elsewhere)])

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
# Current behaviour, not endorsed: the index keys a message by its uuid, and
# a user message has none, so each is stored under the id "unknown", which
# the whole index shares, and only the first one ever is kept. The answers
# carry uuids, so they show both turns reached the index.
answers = [any(text.strip() == want for text in texts)
           for want in ("TOOL_RESULT_OK", "flood line\nflood line")]
report("every indexed message belongs to that session, the answer of each turn included",
       owners == [first_id] and all(answers),
       "owners: %s; answers indexed: %s" % (owners, answers))

requests = [line["messages"] for line in read_lines(messages_log)]
streams = sorted((c for c in read_lines(connections_log) if c.get("stream")),
                 key=lambda c: c["started"])
report("the model saw three streamed requests",
       len(requests) == 3 and len(streams) == 3,
       "%d logged with messages, %d streamed" % (len(requests), len(streams)))

# What each request adds to the one before it: the reply of the fake to that one,
# then the next user message.
added = [
    [("assistant", [("tool_use", "toolu_fake_1", "Bash")]), ("user", [("tool_result", "toolu_fake_1")])],
    [("assistant", [("text", "TOOL_RESULT_OK")]), ("user", [("text", text_prompt)])],
]
carried = len(requests) == 3 and signature(requests[0]) == [("user", [("text", tool_prompt)])]
for (before, after), want in zip(zip(requests, requests[1:]), added):
    before, after = without_cache_control(before), without_cache_control(after)
    if after[: len(before)] != before or signature(after[len(before):]) != want:
        carried = False
report("each request carries the one before it, the reply to it and the next user message",
       carried, [signature(r) for r in requests])

label = "the request of the second turn came on a connection the first turn used"
if len(streams) == 3:
    first_turn = sorted({streams[0]["connection"], streams[1]["connection"]})
    report(label, streams[2]["connection"] in first_turn,
           "first turn: %s; second turn: %s" % (first_turn, streams[2]["connection"]))
else:
    report(label, False, streams)

saved = []
if len(files) == 1:
    saved = load(files[0]).get("messages") or []
answer = ("assistant", [("text", "flood line\nflood line")])
expected = signature(requests[-1]) + [answer] if requests else None
report("the saved session is the conversation the model saw, with its last answer",
       signature(saved) == expected, "saved: %s; expected: %s" % (signature(saved), expected))
print("end\t\t")
' "$1" "$2" "$3" "$4" "$5" "$ORT_TOOL_PROMPT" "$ORT_TEXT_PROMPT" "${6:-}"
}

# Turn the checker's lines into results. Sets ORT_ID from an "id" line.
_ort_report() { # renderer, then the checker's arguments
  local r="$1" verdict label seen other="" ended=0
  shift
  while IFS=$'\t' read -r verdict label seen; do
    case "$verdict" in
      ok) _pass "$r: $label" ;;
      FAIL) _fail "$r: $label" "$seen" ;;
      id) ORT_ID="$label" ;;
      end) ended=1 ;;
      *) other+="$verdict"$'\n' ;;
    esac
  done < <(_ort_check "$@" 2>&1)
  if [ -n "$other" ] || [ "$ended" -ne 1 ]; then
    _fail "$r: the checker runs to the end" "${other:-no end line}"
  fi
}

# Wait until the pane shows a horizontal rule this wide, which only a frame
# drawn at that width has.
_ort_wait_rule() { # width
  local waited=0
  while [ "$waited" -lt 100 ]; do
    tui_capture | python3 -B -c 'import re, sys
runs = [len(m) for m in re.findall("\u2500+", sys.stdin.read())]
sys.exit(0 if max(runs, default=0) >= int(sys.argv[1]) else 1)' "$1" && return 0
    sleep 0.2; waited=$((waited + 1))
  done
  return 1
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
    _ort_report "$r" final "$dir/home" "$dir/project" "$dir/messages.log" "$dir/connections.log" \
      "$ORT_ID"
  fi
  tui_stop
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
}

# Returns nonzero when the journey cannot reach the final checks.
_ort_turns() { # renderer, dir
  local r="$1" dir="$2" log="$2/messages.log" running
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
  _ort_report "$r" first "$dir/home" "$dir/project" "$log" "$dir/connections.log"
  if [ -z "$ORT_ID" ]; then return 1; fi

  # A redraw: wider than the pane was, so a rule that wide can only come from
  # a frame drawn after the resize. tmux holds a resize for 250 ms.
  local wide=$((TUI_WIDTH + 20))
  _tmux resize-window -t "$TUI_SESSION" -x "$wide" -y $((TUI_HEIGHT + 5))
  if _ort_wait_rule "$wide"; then _pass "$r: the app draws a frame at the new size"
  else _fail "$r: the app draws a frame at the new size" "$(tui_capture)"; return 1; fi
  _tmux resize-window -t "$TUI_SESSION" -x "$TUI_WIDTH" -y "$TUI_HEIGHT"
  sleep 0.6
  if wait_for "TOOL_RESULT_OK" 20 && \
    [ "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" = "$running" ]; then
    _pass "$r: back at the suite size, the app still runs and the first answer is on screen"
  else
    _fail "$r: back at the suite size, the app still runs and the first answer is on screen" \
      "$(tui_capture)"
    return 1
  fi

  tui_type "$ORT_TEXT_PROMPT"; sleep 0.3; tui_keys Enter
  if ! _ort_wait_for_requests "$log" 3 || ! wait_for "flood line" 15 || ! _ort_wait_idle; then
    _fail "$r: the second turn ends with the model's answer" \
      "$(_ort_lines "$log") requests; $(tui_capture)"
    return 1
  fi
  if ! _ort_wait_saved "$dir/home" "flood line"; then
    _fail "$r: the second turn is saved" "$(find "$dir/home" -type f)"; return 1
  fi
  _pass "$r: the second turn ends and is saved"
  tui_quit_and_check "$r: Ctrl+C twice after the turns" C-c C-c
  # The checks that follow are about the run as it ended.
  [ "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" != "$running" ]
}
