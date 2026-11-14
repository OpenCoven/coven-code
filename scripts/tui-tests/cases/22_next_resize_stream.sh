#!/usr/bin/env bash
# shellcheck shell=bash
#
# Resizing while a turn streams (#417). A burst of text deltas arrives and the
# stream stays open while the terminal changes size, including sizes too small
# to draw anything.
# Nothing may be lost, the draft stays as typed and the terminal comes back whole.

register_case tc_next_resize_stream

_resize_saved_count() { # home
  find "$1" -type f -path '*/sessions/*.json' -exec grep -o 'flood line' {} + 2>/dev/null \
    | wc -l | tr -d ' '
}

tc_next_resize_stream() {
  describe "Next: resizing while a turn streams"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/project" "$scratch/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"

  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$scratch/port" "$scratch/requests.log" "$scratch/markers" 2>/dev/null &
  local fake_pid=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -s "$scratch/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" var
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/project") && env"
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$(cat "$scratch/port" 2>/dev/null)")"
  TUI_BOOT_STRING=preview

  if [ ! -s "$scratch/port" ]; then
    _fail "the fake provider started" "no port file in $scratch after 5s"
  elif tui_start; then
    # The fake sends every delta, then keeps the stream open until released,
    # so each resize below lands while the turn is still running.
    local count=150000 running size want got reached="" missing="" screen rows
    running="$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')"
    tui_type "flood: $count hold"; sleep 0.3; tui_keys Enter
    if wait_for "flood line" 20; then _pass "the turn is streaming"
    else _fail "the turn is streaming" "$(tui_capture)"; fi

    tui_type "kept draft"
    # tmux holds a pane resize for 250 ms and merges the ones that arrive
    # meanwhile, so each size is kept past that before the next.
    for size in "80 24" "40 10" "200 50" "20 5" "120 40" "1 1" "2 2" "160 30" "10 3" "80 24" "120 40"; do
      want="${size% *}x${size#* }"
      _tmux resize-window -t "$TUI_SESSION" -x "${size% *}" -y "${size#* }"
      got="$(_tmux display-message -p -t "$TUI_SESSION" '#{window_width}x#{window_height}')"
      if [ "$got" = "$want" ]; then reached+=" $want"
      else info "tmux left the window at $got instead of $want"; fi
      sleep 0.6
    done
    for want in 200x50 20x5 1x1 2x2 10x3 120x40; do
      case " $reached " in *" $want "*) ;; *) missing+=" $want" ;; esac
    done
    if [ -z "$missing" ]; then _pass "tmux applied every extreme size, each past its resize delay"
    else _fail "tmux applied every extreme size, each past its resize delay" \
      "not applied:$missing (applied:$reached)"; fi

    assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" "$running" \
      "the process survives every size"
    # A session is saved when its turn ends, so nothing saved yet means every
    # size reached the app while the turn was open.
    assert_eq "$(_resize_saved_count "$scratch/home")" "0" "the turn is still open after the last size"
    # Laying out the whole transcript again at a new width can leave the pane
    # blank for a while in a debug build; wait for a drawn frame.
    if wait_for "working" 120; then _pass "the open turn is drawn at the final size"
    else _fail "the open turn is drawn at the final size" "$(tui_capture)"; fi

    : >"$scratch/markers/release"
    if wait_for "ready" 240 && wait_for "kept draft" 30; then _pass "the turn finishes"
    else _fail "the turn finishes" "$(tui_capture)"; fi
    screen="$(tui_capture)"
    got="$(_tmux display-message -p -t "$TUI_SESSION" '#{window_width}x#{window_height}')"
    # The last drawn row is the composer: a sent draft would leave it empty.
    assert_contains "$(tail -n 1 <<<"${screen:-<blank pane at $got>}")" "kept draft" \
      "the draft is as typed, still in the composer"
    assert_contains "${screen:-<blank pane at $got>}" "flood line" "the answer is on screen"
    assert_absent "$screen" "panicked" "nothing panicked"

    # The last row of the frame is the composer at the size the terminal has now.
    rows="$(grep -c '' <<<"$screen")"
    assert_eq "$rows" "40" "the frame fills the final height"

    # The turn is saved when it ends; the count proves no delta was dropped.
    waited=0
    while [ "$waited" -lt 300 ] && [ "$(_resize_saved_count "$scratch/home")" -lt "$count" ]; do
      sleep 0.1; waited=$((waited + 1))
    done
    assert_eq "$(_resize_saved_count "$scratch/home")" "$count" "every delta reached the saved turn"
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"; TUI_BOOT_STRING="$saved_boot"
  kill "$fake_pid" 2>/dev/null; wait "$fake_pid" 2>/dev/null
  rm -rf "$scratch"
}
