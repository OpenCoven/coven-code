#!/usr/bin/env bash
# shellcheck shell=bash
#
# A restarted Coven daemon is not shown as the one this session registered
# with (#412, reconnect policy B), in Classic and in Next. A fake daemon
# listens on coven.sock in a disposable COVEN_HOME: its health names its own
# pid and start time, as Coven's does, it accepts ledger registrations, and it
# logs every request line when it arrives, with the session id a registration
# carries, and again once it is answered. With `daemonLedger` on, the session
# registers with the first fake, which has answered the registration and the
# health read after it before it is stopped. A second fake, a new process,
# then starts on the same socket path.
#
# Within 30 s of the restart, with no input (the reachability cache holds a
# reading for 10 s and the event loop re-reads it every second), the daemon
# state must read unknown: Next's header says `ledger unknown` instead of
# `ledger-registered`; Classic's footer shows `◌ coven (unknown)` instead of
# the online glyph and its welcome line says `unknown` instead of `online`.
# While it is unknown, and across at least two more probes of the new daemon,
# nothing registers with it. A session start does: after one turn against
# `fake_anthropic.py` has saved the session, `/session resume <id>` binds it
# and the second fake receives a registration for that id. It holds its
# answer for 2 s: meanwhile Classic still reads unknown and Next reads
# `registering`. Once it answers, the state reads registered and online
# again, and a second later there is still exactly one registration. Classic
# runs with COVEN_CODE_UI unset; needs `python3`.

register_case tc_daemon_restart_unknown

_dru_start_daemon() { # socket path, ready file, request log, seconds to hold a registration -> sets DRU_PID; 0 once listening
  rm -f "$2"
  python3 - "$1" "$2" "$3" "$4" <<'PY' >/dev/null 2>&1 &
import datetime, json, os, socket, sys, time
path, ready, log, hold = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])
health = json.dumps({
    "ok": True, "apiVersion": "coven.daemon.v1", "covenVersion": "0.5.0",
    "daemon": {
        "pid": os.getpid(),
        "startedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "socket": path,
    },
})
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
        head, _, body = request.partition(b"\r\n\r\n")
        lines = head.split(b"\r\n")
        length = 0
        for header in lines[1:]:
            name, _, value = header.partition(b":")
            if name.strip().lower() == b"content-length":
                length = int(value)
        while len(body) < length:
            chunk = conn.recv(4096)
            if not chunk:
                break
            body += chunk
        line = lines[0].decode("utf-8", "replace")
        try:
            session = json.loads(body).get("id", "")
        except (ValueError, AttributeError):
            session = ""
        with open(log, "a", encoding="utf-8") as out:
            out.write(f"{line} id={session}\n")
        if line.startswith("GET /api/v1/health "):
            answer = health
        elif line.startswith("POST "):
            answer = "{}"
            if line.startswith("POST /api/v1/sessions/external "):
                time.sleep(hold)
        else:
            answer = "[]"
        payload = answer.encode()
        conn.sendall(b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n"
                     b"Content-Length: %d\r\n\r\n" % len(payload) + payload)
        with open(log, "a", encoding="utf-8") as out:
            out.write(f"answered {line}\n")
    except (OSError, ValueError):
        pass
    conn.close()
PY
  DRU_PID=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -e "$2" ]; do sleep 0.1; waited=$((waited + 1)); done
  [ -e "$2" ]
}

_dru_stop_daemon() { # socket path
  if [ -n "${DRU_PID:-}" ]; then
    kill "$DRU_PID" 2>/dev/null
    wait "$DRU_PID" 2>/dev/null
    DRU_PID=""
  fi
  rm -f "$1"
}

# How many logged lines start with this text.
_dru_count() { # request log, line prefix
  if [ -f "$1" ]; then awk -v p="$2" 'index($0, p) == 1 { n++ } END { print n + 0 }' "$1"
  else echo 0; fi
}

# How many health reads were answered after the first answered registration.
_dru_health_after_registration() { # request log
  if [ -f "$1" ]; then
    awk 'index($0, "answered POST /api/v1/sessions/external ") == 1 { seen = 1; next }
         seen && index($0, "answered GET /api/v1/health ") == 1 { n++ }
         END { print n + 0 }' "$1"
  else echo 0; fi
}

# Wait until the log holds at least this many lines with this start.
_dru_wait_for_requests() { # request log, line prefix, count, seconds
  local waited=0
  while [ "$waited" -lt $(($4 * 5)) ]; do
    [ "$(_dru_count "$1" "$2")" -ge "$3" ] && return 0
    sleep 0.2; waited=$((waited + 1))
  done
  return 1
}

# Wait until the turn no longer shows as running.
_dru_wait_idle() {
  local waited=0
  while [ "$waited" -lt 50 ]; do
    grep -qF -- "$DRU_BUSY" <<<"$(tui_capture)" || return 0
    sleep 0.2; waited=$((waited + 1))
  done
  return 1
}

# Start the TUI for one renderer against the fake API, with its own home.
_dru_launch() { # renderer, home, port, project, coven home
  local var ui=""
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$4") && env"
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI \
    COVEN_SESSION_SOURCE; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  if [ "$1" = next ]; then
    ui="COVEN_CODE_UI=next"; TUI_BOOT_STRING="preview"; DRU_BUSY="working"
    DRU_REGISTERED="ledger-registered"; DRU_UNKNOWN="ledger unknown"
    DRU_RENEWING="engine-local, registering"
  else
    TUI_BOOT_STRING="Coven v"; DRU_BUSY="… · "
    DRU_REGISTERED="✦ coven"; DRU_UNKNOWN="◌ coven (unknown)"
    DRU_RENEWING="$DRU_UNKNOWN"
  fi
  # shellcheck disable=SC2086 # an empty selector adds nothing
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$2" "COVEN_HOME=$5" $ui \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$3")"
  tui_start
}

tc_daemon_restart_unknown() {
  describe "A restarted daemon reads as unknown until the next session start"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" r
  DRU_PID=""
  for r in classic next; do
    # A short directory name: the socket path must stay under the platform's
    # limit (104 bytes on macOS) below a long TMPDIR.
    _dru_journey "$r" "$scratch/${r:0:1}"
  done
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

# One renderer's journey, with a fake API and fake daemons of its own.
_dru_journey() { # renderer, dir
  local r="$1" dir="$2" waited=0 fake_pid
  local sock="$2/coven/coven.sock" ready="$2/ready"
  mkdir -p "$dir/home/.coven-code" "$dir/coven" "$dir/p" "$dir/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true, "daemonLedger": true}' \
    >"$dir/home/.coven-code/settings.json"
  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$dir/port" "$dir/requests.log" "$dir/markers" 2>/dev/null &
  fake_pid=$!
  while [ "$waited" -lt 50 ] && [ ! -s "$dir/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  if [ ! -s "$dir/port" ]; then
    _fail "$r: the fake API starts"
  elif ! _dru_start_daemon "$sock" "$ready" "$dir/daemon-1.log" 0; then
    _fail "$r: the fake daemon listens" "no socket at $sock (a path of ${#sock} characters)"
  else
    _dru_launch "$r" "$dir/home" "$(cat "$dir/port")" "$dir/p" "$dir/coven" && _dru_steps "$r" "$dir"
  fi
  tui_stop
  _dru_stop_daemon "$sock"
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
}

_dru_steps() { # renderer, dir
  local r="$1" first="$2/daemon-1.log" second="$2/daemon-2.log" s id waited=0
  local sock="$2/coven/coven.sock" register="POST /api/v1/sessions/external "

  if wait_for "$DRU_REGISTERED" 15; then _pass "$r: the session registers and reads '$DRU_REGISTERED'"
  else _fail "$r: the session registers and reads '$DRU_REGISTERED'" "$(tui_capture)"; return 0; fi
  if ! _dru_wait_for_requests "$first" "$register" 1 10; then
    _fail "$r: the first daemon took one registration" "$(cat "$first" 2>/dev/null)"; return 0
  fi
  assert_eq "$(_dru_count "$first" "$register")" "1" "$r: the first daemon took one registration"
  id="$(grep -- "^$register" "$first" | sed 's/.* id=//')"
  if [ -z "$id" ]; then
    _fail "$r: the registration carries the session id" "$(cat "$first")"; return 0
  fi
  # The registration is settled only after the health read that follows it:
  # stopping the fake before that would leave the state unknown from launch.
  # The first health answer after the POST may be the background probe's,
  # so give the registration's own read time to follow (it is sent as soon
  # as the POST is answered), then require every request to be answered.
  while [ "$waited" -lt 50 ] && [ "$(_dru_health_after_registration "$first")" -lt 1 ]; do
    sleep 0.2; waited=$((waited + 1))
  done
  sleep 1.5
  if [ "$(_dru_health_after_registration "$first")" -lt 1 ] \
    || [ "$(_dru_count "$first" "answered ")" -ne "$(($(wc -l <"$first") / 2))" ]; then
    _fail "$r: the first daemon answered the health read after the registration" "$(cat "$first")"
    return 0
  fi
  assert_absent "$(tui_capture)" "$DRU_UNKNOWN" "$r: the state is not unknown before the restart"

  # The daemon is replaced: a new process, so a new pid and start time.
  _dru_stop_daemon "$sock"
  if ! _dru_start_daemon "$sock" "$2/ready" "$second" 2; then
    _fail "$r: the second fake daemon listens" "no socket at $sock"; return 0
  fi
  if wait_for "$DRU_UNKNOWN" 30; then _pass "$r: after the restart the state reads '$DRU_UNKNOWN'"
  else _fail "$r: after the restart the state reads '$DRU_UNKNOWN'" "$(tui_capture)"; return 0; fi
  s="$(tui_capture)"
  assert_absent "$s" "$DRU_REGISTERED" "$r: the restarted daemon is not shown as '$DRU_REGISTERED'"
  if [ "$r" = classic ]; then
    assert_matches "$s" "(· |Daemon: )unknown" "$r: the welcome line says unknown"
    assert_absent "$s" "· online" "$r: the welcome line does not say online"
    assert_absent "$s" "Daemon: online" "$r: the short welcome line does not say online"
  fi

  # Nothing registers between session starts: the new daemon is probed at
  # least twice more and receives no registration.
  if _dru_wait_for_requests "$second" "GET /api/v1/health " \
    "$(($(_dru_count "$second" "GET /api/v1/health ") + 2))" 40; then
    assert_eq "$(_dru_count "$second" "$register")" "0" \
      "$r: nothing registers with the new daemon before a session start"
    assert_contains "$(tui_capture)" "$DRU_UNKNOWN" "$r: the state is still unknown"
  else
    _fail "$r: the new daemon keeps being probed" "$(cat "$second" 2>/dev/null)"; return 0
  fi

  # One turn, so the session is saved, then a session start.
  tui_type "flood: 1"; sleep 0.3; tui_keys Enter
  if ! wait_for "flood line" 15 || ! _dru_wait_idle; then
    _fail "$r: a turn runs while the daemon state is unknown" "$(tui_capture)"; return 0
  fi
  assert_eq "$(_dru_count "$second" "$register")" "0" "$r: a turn does not register"
  tui_type "/session resume $id"; sleep 0.3; tui_keys Enter
  # The new daemon has the registration and holds its answer for 2 s.
  if ! _dru_wait_for_requests "$second" "$register" 1 10; then
    _fail "$r: the session start sends a registration to the new daemon" "$(tui_capture)"; return 0
  fi
  s="$(tui_capture)"
  if [ "$(_dru_count "$second" "answered $register")" = "0" ]; then
    assert_contains "$s" "$DRU_RENEWING" "$r: until the new daemon answers, the state reads '$DRU_RENEWING'"
    assert_absent "$s" "$DRU_REGISTERED" "$r: until the new daemon answers, it is not shown as '$DRU_REGISTERED'"
  else
    _fail "$r: the new daemon was still holding its answer when the screen was read" "$(cat "$second")"
  fi
  if wait_for "$DRU_REGISTERED" 15; then _pass "$r: the next session start reads '$DRU_REGISTERED' again"
  else _fail "$r: the next session start reads '$DRU_REGISTERED' again" "$(tui_capture)"; fi
  # A loop tick later there is still one registration, for that session.
  sleep 1.5
  assert_eq "$(grep -- "^$register" "$second" 2>/dev/null | sed 's/.* id=//')" "$id" \
    "$r: the session start registered that session, once, with the new daemon"
  assert_absent "$(tui_capture)" "$DRU_UNKNOWN" "$r: the state no longer reads unknown"
}
