#!/usr/bin/env bash
# shellcheck shell=bash
#
# A second approval key and the next queued panel (#411). One response from a
# local fake of the Messages API asks for two Bash calls, so the second call's
# request waits in the queue behind the first panel. The first panel is decided
# with y and a second y follows: in the same write, at once, and 150 ms later,
# once the next panel is open. Each round checks whether the second call ran
# (its marker file), whether its panel is still open, and what the model was
# told, then decides that panel on purpose.
#
# A key that is already waiting when the first panel closes reaches the
# composer as text in both renderers: the loop reads it before it opens the
# next request. A key that reaches the next panel decides nothing in Next,
# which ignores keys other than Escape until a new panel has been quiet for
# 400 ms. Classic takes it at once and decides the panel. The case pins both;
# whether Classic should change is a decision for the maintainer.
# Engine-local evidence only.

register_case tc_approval_queue

# Next shows this while a new panel still ignores keys.
QUEUE_ARMING="keys typed before this appeared are ignored"

_queue_requests() {
  if [ -f "$QUEUE_LOG" ]; then wc -l <"$QUEUE_LOG" | tr -d ' '; else echo 0; fi
}

# What the model was told in the request with this number, waiting up to 10 s
# for it: "ok" or "error" for each tool result in order, "none" when that
# request carries no result, or "missing" when it never came.
_queue_told() { # request number
  local waited=0
  while [ "$waited" -lt 100 ] && [ "$(_queue_requests)" -lt "$1" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  python3 -B -c 'import json, sys
lines = open(sys.argv[1], encoding="utf-8").read().splitlines()
n = int(sys.argv[2])
if len(lines) < n:
    print("missing"); sys.exit()
content = json.loads(lines[n - 1])["last"].get("content")
blocks = [b for b in content if isinstance(b, dict) and b.get("type") == "tool_result"] \
    if isinstance(content, list) else []
print(" ".join("error" if b.get("is_error") else "ok" for b in blocks) or "none")' \
    "$QUEUE_LOG" "$1"
}

# The panel for the call that runs `touch NAME` is on screen. Next shows the
# command on a row of its own; Classic shows it after a prompt glyph. Neither
# matches the prompt's JSON or a tool row.
_queue_panel_shows() { # ui, name
  local s; s="$(tui_capture)"
  if [ "$1" = next ]; then grep -qE "^touch $2 *\$" <<<"$s"
  else grep -qF "❯ touch $2 " <<<"$s"; fi
}

_queue_wait_panel() { # ui, name
  local waited=0
  while [ "$waited" -lt 150 ]; do
    _queue_panel_shows "$1" "$2" && return 0
    sleep 0.1; waited=$((waited + 1))
  done
  return 1
}

# The stray y is in the composer, as text.
_queue_in_composer() { grep -qE '^(›|❯) y' <<<"$(tui_capture)"; }

_queue_wait_file() { # path
  local waited=0
  while [ "$waited" -lt 100 ] && [ ! -e "$1" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  [ -e "$1" ]
}

# Press y twice, GAP seconds apart, each as its own write, and print how many
# milliseconds passed from the first press to the second's arrival.
_queue_press_twice() { # gap
  python3 -B -c 'import subprocess, sys, time
socket, session, gap = sys.argv[1], sys.argv[2], float(sys.argv[3])
press = ["tmux", "-L", socket, "send-keys", "-t", session, "y"]
start = time.monotonic()
subprocess.run(press, check=True)
time.sleep(gap)
subprocess.run(press, check=True)
print(int((time.monotonic() - start) * 1000))' "$TUI_SOCKET" "$TUI_SESSION" "$1"
}

# Wait until the second y has been handled: its call ran, it is in the
# composer, or (Next) the open panel has gone quiet, so it was ignored.
_queue_settle() { # ui, marker
  local waited=0 s
  while [ "$waited" -lt 100 ]; do
    [ -e "$2" ] && return 0
    s="$(tui_capture)"
    grep -qE '^(›|❯) y' <<<"$s" && return 0
    if [ "$1" = next ] && _queue_panel_shows next "${2##*/}" && \
      ! grep -qF -- "$QUEUE_ARMING" <<<"$s"; then
      return 0
    fi
    sleep 0.1; waited=$((waited + 1))
  done
}

# One round: decide the first panel with y, then press a second y.
_queue_round() { # ui, round number, how the second y is pressed
  local ui="$1" how="$3" project="$QUEUE_SCRATCH/project"
  local first="$ui-$2-first" second="$ui-$2-second"
  local tag="$ui, $how" before sent_ms=0 path decided
  before="$(_queue_requests)"
  tui_type "tools: [[\"Bash\",{\"command\":\"touch $first\"}],[\"Bash\",{\"command\":\"touch $second\"}]]"
  tui_keys Enter
  if ! _queue_wait_panel "$ui" "$first"; then
    _fail "$tag: the first call asks first" "$(tui_capture)"
    return 0
  fi
  if [ "$ui" = next ]; then wait_absent "$QUEUE_ARMING" 5 >/dev/null; fi

  case "$how" in
    "same write") tui_paste "yy" ;;
    "at once") sent_ms="$(_queue_press_twice 0)" ;;
    "150 ms later") sent_ms="$(_queue_press_twice 0.15)" ;;
  esac
  if ! [[ "$sent_ms" =~ ^[0-9]+$ ]]; then
    _fail "$tag: both keys are sent" "sender printed: '$sent_ms'"
    sent_ms=0
  fi
  if _queue_wait_file "$project/$first"; then
    _pass "$tag: the first y decides the first panel and its call runs"
  else
    _fail "$tag: the first y decides the first panel and its call runs" "$(tui_capture)"
  fi
  _queue_settle "$ui" "$project/$second"
  if _queue_in_composer; then path=composer; else path=panel; fi
  if [ -e "$project/$second" ]; then decided=yes; else decided=no; fi
  log "       $tag: second y sent ${sent_ms} ms after the first, reached the $path, decided: $decided"

  if [ "$ui" = next ]; then
    # Next counts quiet from when it reads a key, not from when the key
    # arrived, so a loop stalled for 400 ms after opening the next panel could
    # still let an earlier key count. The limit below bounds only the sending.
    if [ "$sent_ms" -ge 400 ]; then
      # A key this late can find the next panel armed, and then it counts.
      _skip "$tag: the second y took ${sent_ms} ms to send, so this round proves nothing"
    else
      assert_eq "$decided" "no" "$tag: the second y, which reached the $path, does not decide the next panel"
    fi
    case "$how" in
      "same write")
        assert_eq "$path" "composer" "$tag: the second y reaches the composer as text" ;;
      "150 ms later")
        # With a panel open, a key that is not in the composer went to the panel.
        if [ "$path" = panel ]; then _pass "$tag: the second y missed the composer, so the open panel took it"
        else _skip "$tag: the second y arrived before the next panel opened"; fi ;;
    esac
  else
    case "$how" in
      "same write")
        assert_eq "$path/$decided" "composer/no" \
          "$tag: the second y reaches the composer and decides nothing" ;;
      "at once")
        # Timing decides where it lands; whichever it reached must agree.
        if [ "$path" = panel ]; then
          assert_eq "$decided" "yes" "$tag: the second y reached the next panel and decided it"
        else
          assert_eq "$decided" "no" "$tag: the second y reached the composer and decided nothing"
        fi ;;
      "150 ms later")
        if [ "$path" = panel ]; then
          assert_eq "$decided" "yes" \
            "$tag: a y on the open next panel decides it at once (differs from Next by design)"
        else
          _skip "$tag: the second y arrived before the next panel opened"
          assert_eq "$decided" "no" "$tag: the second y reached the composer and decided nothing"
        fi ;;
    esac
  fi

  if [ "$decided" = no ]; then
    if _queue_wait_panel "$ui" "$second"; then _pass "$tag: the next panel is still open"
    else _fail "$tag: the next panel is still open" "$(tui_capture)"; fi
    assert_eq "$(_queue_requests)" "$((before + 1))" \
      "$tag: the model has been told nothing while the next call waits"
    if [ "$ui" = next ]; then wait_absent "$QUEUE_ARMING" 5 >/dev/null; fi
    tui_keys y
    if _queue_wait_file "$project/$second"; then
      _pass "$tag: a deliberate y then runs the second call"
    else
      _fail "$tag: a deliberate y then runs the second call" "$(tui_capture)"
    fi
  fi
  assert_eq "$(_queue_told $((before + 2)))" "ok ok" "$tag: the model hears that both calls ran"
  assert_eq "$(_queue_requests)" "$((before + 2))" "$tag: no other request was made"

  # Back to an idle turn and an empty composer, ready for the next round. The
  # status line said something else while the round ran; Classic's idle line
  # starts with a verb picked at random and always ends "· done".
  if [ "$ui" = next ]; then wait_for "ready" 10 >/dev/null; else wait_for "· done" 10 >/dev/null; fi
  local waited=0
  while [ "$waited" -lt 50 ] && _queue_in_composer; do
    tui_keys C-u; sleep 0.2; waited=$((waited + 1))
  done
}

_queue_launch() { # ui, boot
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$QUEUE_SCRATCH/project") && env"
  local var
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$QUEUE_SCRATCH/home" "COVEN_HOME=$QUEUE_SCRATCH/home/.coven" "COVEN_CODE_UI=$1" \
    ANTHROPIC_API_KEY=fake-key \
    "ANTHROPIC_BASE_URL=http://127.0.0.1:$(cat "$QUEUE_SCRATCH/port")")"
  TUI_BOOT_STRING="$2"
  tui_start
}

tc_approval_queue() {
  describe "A second approval key and the next queued panel"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  QUEUE_SCRATCH="$(mktemp -d)"
  QUEUE_LOG="$QUEUE_SCRATCH/requests.log"
  mkdir -p "$QUEUE_SCRATCH/home/.coven-code" "$QUEUE_SCRATCH/project" "$QUEUE_SCRATCH/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$QUEUE_SCRATCH/home/.coven-code/settings.json"

  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$QUEUE_SCRATCH/port" "$QUEUE_LOG" "$QUEUE_SCRATCH/markers" 2>"$QUEUE_SCRATCH/fake.err" &
  local fake_pid=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -s "$QUEUE_SCRATCH/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  if [ ! -s "$QUEUE_SCRATCH/port" ]; then
    _fail "the fake API starts"
  else
    local ui boot round how
    for ui in next classic; do
      if [ "$ui" = next ]; then boot=preview; else boot="Coven v"; fi
      if _queue_launch "$ui" "$boot"; then
        round=0
        for how in "same write" "at once" "150 ms later"; do
          round=$((round + 1))
          _queue_round "$ui" "$round" "$how"
        done
        tui_keys C-c; sleep 0.3; tui_keys C-c
      fi
      tui_stop
    done
  fi

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
  # A prompt the fake could not read shows up here, not on screen.
  if grep -q "Traceback" "$QUEUE_SCRATCH/fake.err" 2>/dev/null; then
    _fail "the fake API handled every request" "$(cat "$QUEUE_SCRATCH/fake.err")"
  fi
  rm -rf "$QUEUE_SCRATCH"
  unset QUEUE_SCRATCH QUEUE_LOG
}
