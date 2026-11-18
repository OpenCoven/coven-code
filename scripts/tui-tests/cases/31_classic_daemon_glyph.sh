#!/usr/bin/env bash
# shellcheck shell=bash
#
# Classic's footer follows the Coven daemon (#513). A fake daemon listens on
# coven.sock in a disposable COVEN_HOME and answers every request. The footer
# shows the online glyph while it runs; after the fake stops and its socket is
# removed, as `coven daemon stop` leaves it, the footer turns to the offline
# glyph without any input; after a fresh fake starts, it turns back. Each
# change must land within 25 s: the reachability cache holds a reading for
# 10 s, and the event loop re-reads it every second. Classic runs with
# COVEN_CODE_UI unset and no provider; needs `python3`.

register_case tc_classic_daemon_glyph

_dg_start_daemon() { # socket path, ready file -> sets DG_PID; 0 once listening
  rm -f "$2"
  python3 - "$1" "$2" <<'PY' >/dev/null 2>&1 &
import socket, sys
path, ready = sys.argv[1], sys.argv[2]
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(path)
server.listen(16)
open(ready, "w").close()
while True:
    conn, _ = server.accept()
    conn.settimeout(2)
    request = b""
    try:
        while b"\r\n\r\n" not in request:
            chunk = conn.recv(4096)
            if not chunk:
                break
            request += chunk
        conn.sendall(b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n"
                     b"Content-Length: 2\r\n\r\n[]")
    except OSError:
        pass
    conn.close()
PY
  DG_PID=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -e "$2" ]; do sleep 0.1; waited=$((waited + 1)); done
  [ -e "$2" ]
}

_dg_stop_daemon() { # socket path
  if [ -n "${DG_PID:-}" ]; then
    kill "$DG_PID" 2>/dev/null
    wait "$DG_PID" 2>/dev/null
    DG_PID=""
  fi
  rm -f "$1"
}

tc_classic_daemon_glyph() {
  describe "Classic daemon glyph follows the daemon (COVEN_CODE_UI unset)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/home/.coven" "$scratch/project"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  local sock="$scratch/home/.coven/coven.sock" ready="$scratch/ready"
  DG_PID=""

  if ! _dg_start_daemon "$sock" "$ready"; then
    _fail "the fake daemon listens" "no socket at $sock"
    _dg_stop_daemon "$sock"; rm -rf "$scratch"; return 0
  fi

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" var
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/project") && env"
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN \
    COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven")"
  TUI_BOOT_STRING="Coven v"

  if tui_start; then
    if wait_for "✦ coven" 10; then _pass "the footer shows the online glyph while the daemon runs"
    else _fail "the footer shows the online glyph while the daemon runs" "$(tui_capture)"; fi

    _dg_stop_daemon "$sock"
    if wait_for "○ coven" 25; then _pass "the footer turns offline after the daemon stops"
    else _fail "the footer turns offline after the daemon stops" "$(tui_capture)"; fi
    assert_absent "$(tui_capture)" "✦ coven" "the online glyph is gone"

    if _dg_start_daemon "$sock" "$ready"; then
      if wait_for "✦ coven" 25; then _pass "the footer turns online after the daemon starts again"
      else _fail "the footer turns online after the daemon starts again" "$(tui_capture)"; fi
    else
      _fail "the fake daemon listens again" "no socket at $sock"
    fi
  fi
  tui_stop
  _dg_stop_daemon "$sock"

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}
