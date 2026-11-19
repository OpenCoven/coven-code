#!/usr/bin/env bash
# shellcheck shell=bash
#
# A Coven daemon that accepts every connection and never answers does not
# freeze the terminal (#580), in Classic and in Next. A fake daemon listens
# on coven.sock in a disposable COVEN_HOME: it reads each request, logs its
# request line and stays silent, holding the connection open. The
# COVEN_HOME also holds a familiars.toml naming one familiar, so
# `/familiar list` asks the daemon for live statuses, and `daemonLedger` is
# on, so the session registers at launch and completes at exit.
#
# `/coven sessions`, `/coven familiars` and `/familiar list` each ask the
# daemon. Right after each one, text typed into the prompt is drawn within
# 2.5 s (the request itself waits 5 s), and the toast says the daemon is
# being waited for. Each sends exactly one request to the daemon, and ends
# with its result once the request's 5 s deadline passes, not before 4 s:
# an error for the two `/coven` listings, the familiar list (without live
# statuses) for `/familiar list`. A second listing typed while one is still
# waiting is not sent: the toast says so and the daemon never sees its
# request. Ctrl+C twice then exits within 4.5 s (the exit waits up to 2 s
# for the completion; the rest is margin for a debug build under load), although the daemon never answers the session's
# completion. Classic runs with COVEN_CODE_UI unset and no provider; needs
# `python3`.

register_case tc_silent_daemon_keeps_input

_sdk_start_daemon() { # socket path, ready file, request log -> sets SDK_PID; 0 once listening
  rm -f "$2"
  python3 - "$1" "$2" "$3" <<'PY' >/dev/null 2>&1 &
import socket, sys, threading
path, ready, log = sys.argv[1], sys.argv[2], sys.argv[3]
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(path)
server.listen(64)
open(ready, "w").close()
held = []

def hold(conn):
    request = b""
    try:
        while b"\r\n\r\n" not in request:
            chunk = conn.recv(4096)
            if not chunk:
                return
            request += chunk
        line = request.split(b"\r\n", 1)[0].decode("utf-8", "replace")
        with open(log, "a", encoding="utf-8") as out:
            out.write(line + "\n")
        # Never answer; keep the connection open until the client leaves.
        while conn.recv(4096):
            pass
    except OSError:
        pass

while True:
    conn, _ = server.accept()
    held.append(conn)
    threading.Thread(target=hold, args=(conn,), daemon=True).start()
PY
  SDK_PID=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -e "$2" ]; do sleep 0.1; waited=$((waited + 1)); done
  [ -e "$2" ]
}

_sdk_stop_daemon() { # socket path
  if [ -n "${SDK_PID:-}" ]; then
    kill "$SDK_PID" 2>/dev/null
    wait "$SDK_PID" 2>/dev/null
    SDK_PID=""
  fi
  rm -f "$1"
}

# How many logged request lines start with this text. Only the listings ask
# for `/api/v1/familiars` during the case: the reachability probe's
# `is_online` also sends that path, and nothing calls it here.
_sdk_count() { # request log, line prefix
  if [ -f "$1" ]; then awk -v p="$2" 'index($0, p) == 1 { n++ } END { print n + 0 }' "$1"
  else echo 0; fi
}

# Wait up to this many tenths of a second for the text; sets SDK_TENTHS to
# the time it took. 0 when seen.
_sdk_seen_within() { # text, tenths
  local start now
  start="$(tui_now_tenths)"
  while :; do
    now="$(tui_now_tenths)"
    SDK_TENTHS=$((now - start))
    grep -qF -- "$1" <<<"$(tui_capture)" && return 0
    [ "$SDK_TENTHS" -ge "$2" ] && return 1
    sleep 0.1
  done
}

# One listing: the prompt keeps taking input while it waits, then its result lands.
_sdk_listing() { # renderer, command, marker, result text, request log, request line
  local r="$1" cmd="$2" marker="$3" result="$4" log="$5" line="$6" before sent
  before="$(_sdk_count "$log" "$line")"
  tui_type "$cmd"; sleep 0.3; tui_keys Enter
  sent="$(tui_now_tenths)"
  tui_type "$marker"
  if _sdk_seen_within "$marker" 25; then
    _pass "$r: after $cmd, typed text is drawn within 2.5 s (about $((SDK_TENTHS / 10)).$((SDK_TENTHS % 10)) s)"
  else
    _fail "$r: after $cmd, typed text is drawn within 2.5 s" "$(tui_capture)"
  fi
  if _sdk_seen_within "Waiting for Coven" 20; then
    _pass "$r: while $cmd waits, the toast says the daemon is being waited for"
  else
    _fail "$r: while $cmd waits, the toast says the daemon is being waited for" "$(tui_capture)"
  fi
  assert_eq "$(_sdk_count "$log" "$line")" "$((before + 1))" \
    "$r: $cmd sent one request to the daemon"
  if _sdk_seen_within "$result" 120; then
    SDK_TENTHS=$(($(tui_now_tenths) - sent))
    if [ "$SDK_TENTHS" -ge 40 ]; then
      _pass "$r: $cmd ends with its result once the request's deadline passes (after about $((SDK_TENTHS / 10)) s)"
    else
      _fail "$r: $cmd ends with its result once the request's deadline passes" \
        "the result came after $((SDK_TENTHS / 10)).$((SDK_TENTHS % 10)) s, before the 5 s deadline"
    fi
  else
    _fail "$r: $cmd ends with its result once the request's deadline passes" "$(tui_capture)"
  fi
  wait_absent "Waiting for Coven" 3 >/dev/null
  tui_keys C-u; sleep 0.3
}

_sdk_journey() { # renderer, dir
  local r="$1" dir="$2" var ui="" before
  local sock="$dir/coven/coven.sock" log="$dir/daemon.log"
  mkdir -p "$dir/home/.coven-code" "$dir/coven" "$dir/p"
  printf '%s\n' '{"hasCompletedOnboarding": true, "daemonLedger": true}' \
    >"$dir/home/.coven-code/settings.json"
  printf '%s\n' '[[familiar]]' 'id = "wisp"' >"$dir/coven/familiars.toml"
  if ! _sdk_start_daemon "$sock" "$dir/ready" "$log"; then
    _fail "$r: the silent fake daemon listens" "no socket at $sock (a path of ${#sock} characters)"
    _sdk_stop_daemon "$sock"
    return 0
  fi

  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$dir/p") && env"
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN \
    COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI COVEN_SESSION_SOURCE; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  if [ "$r" = next ]; then ui="COVEN_CODE_UI=next"; TUI_BOOT_STRING="preview"
  else TUI_BOOT_STRING="Coven v"; fi
  # shellcheck disable=SC2086 # an empty selector adds nothing
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$dir/home" "COVEN_HOME=$dir/coven" $ui)"

  if tui_start; then
    _sdk_listing "$r" "/coven sessions" "zq1x" "daemon transport error" \
      "$log" "GET /api/v1/sessions "
    _sdk_listing "$r" "/coven familiars" "zq2x" "daemon transport error" \
      "$log" "GET /api/v1/familiars "
    _sdk_listing "$r" "/familiar list" "zq3x" "Coven Familiars (" \
      "$log" "GET /api/v1/familiars "

    # A second listing while one is waiting is not sent.
    before="$(_sdk_count "$log" "GET /api/v1/familiars ")"
    tui_type "/coven sessions"; sleep 0.3; tui_keys Enter
    sleep 0.5
    tui_type "/coven familiars"; sleep 0.3; tui_keys Enter
    if _sdk_seen_within "Not sent; waiting on" 25; then
      _pass "$r: a second listing while one waits says it was not sent"
    else
      _fail "$r: a second listing while one waits says it was not sent" "$(tui_capture)"
    fi
    if wait_for "daemon transport error" 12; then
      _pass "$r: the waiting listing still ends with its result"
    else
      _fail "$r: the waiting listing still ends with its result" "$(tui_capture)"
    fi
    sleep 1
    assert_eq "$(_sdk_count "$log" "GET /api/v1/familiars ")" "$before" \
      "$r: the daemon never saw the second listing's request"

    # The launch registration has long given up (2 s health, 5 s request).
    assert_eq "$(_sdk_count "$log" "POST /api/v1/sessions/external ")" "1" \
      "$r: the session sent its registration at launch"
    tui_quit_and_check "$r: Ctrl+C twice with a silent daemon" C-c C-c
    if [ "${TUI_QUIT_TENTHS:-999}" -le 45 ]; then
      _pass "$r: the exit waits at most 4.5 s for the unanswered completion"
    else
      _fail "$r: the exit waits at most 4.5 s for the unanswered completion" \
        "the exit took $((TUI_QUIT_TENTHS / 10)).$((TUI_QUIT_TENTHS % 10)) s"
    fi
    sleep 0.5
    assert_eq "$(_sdk_count "$log" "POST /api/v1/sessions/")" "2" \
      "$r: the exit sent the session's completion"
  fi
  tui_stop
  _sdk_stop_daemon "$sock"
}

tc_silent_daemon_keeps_input() {
  describe "A daemon that never answers does not freeze the terminal or the exit"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" r
  scratch="$(mktemp -d)"
  SDK_PID=""
  for r in classic next; do
    # A short directory name: the socket path must stay under the platform's
    # limit (104 bytes on macOS) below a long TMPDIR.
    _sdk_journey "$r" "$scratch/${r:0:1}"
  done
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}
