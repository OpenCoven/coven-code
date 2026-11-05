#!/usr/bin/env bash
# shellcheck shell=bash
#
# Appearance preview, save, revert and reset under Next (#409). A preview writes
# nothing; a save changes only the keys it names; a file it cannot read is left
# alone. Classic does not take the command.

register_case tc_appearance

_look_launch() { # ui, boot, scratch
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$3/project") && env"
  local var
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET NO_COLOR; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$3/home" "COVEN_HOME=$3/home/.coven" "COVEN_CODE_UI=$1" \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$(cat "$3/port")")"
  TUI_BOOT_STRING="$2"
  tui_start
}

_look_run() { # command, text to wait for, message
  tui_type "$1"; sleep 0.3; tui_keys Enter
  if wait_for "$2" 8; then _pass "$3"; else _fail "$3" "$(tui_capture)"; fi
}

_look_json() { # file
  python3 -I -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1])),sort_keys=True))' "$1"
}

_look_quit() {
  tui_stop
}

_look_wait_moving() {
  local waited=0
  while [ "$waited" -lt 80 ]; do
    if tui_capture | grep -qE '[◐◓◑◒] working'; then return 0; fi
    sleep 0.1; waited=$((waited + 1))
  done
  return 1
}

# A turn long enough to watch the status row.
_look_turn() {
  tui_type 'tools: [["Bash",{"command":"sleep 3"}]]'; sleep 0.3; tui_keys Enter
  if wait_for "Approve Bash" 15; then sleep 0.8; tui_keys y
  else _fail "next: the command asks first" "$(tui_capture)"; fi
}

tc_appearance() {
  describe "Appearance preview, save and reset"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/project/.coven-code" "$scratch/markers"
  local file="$scratch/home/.coven-code/settings.json"
  local project_file="$scratch/project/.coven-code/settings.json"
  # The version is recorded, so launching does not rewrite the file itself.
  local version seen
  version="$("$COVEN_BIN" --version | grep -oE '[0-9]+[.][0-9]+[.][0-9]+' | head -1)"
  seen="\"lastSeenVersion\": \"$version\""
  printf '{"hasCompletedOnboarding":true,%s,"nextAppearance":{"glyphs":"unicode","laterKey":42},"someFutureKey":{"kept":[1,2]}}\n' \
    "$seen" >"$file"

  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$scratch/port" "$scratch/requests.log" "$scratch/markers" 2>/dev/null &
  local fake_pid=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -s "$scratch/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"

  # Preview, refusals, then a save that keeps what this version does not know.
  if _look_launch next preview "$scratch"; then
    cp "$file" "$scratch/before.json"
    _look_run "/config appearance" "Preview one:" "next: the settings are listed"
    assert_contains "$(tui_capture)" "reduceMotion" "next: the listing includes reduced motion"
    _look_run "/config appearance density compact" "Previewing density = compact" \
      "next: a value previews"
    _look_run "/config appearance hints true" "Previewing hints = true" "next: hints preview"
    assert_contains "$(tui_capture)" "Enter send" "next: the hint row appears at once"
    if cmp -s "$file" "$scratch/before.json"; then _pass "next: a preview writes nothing"
    else _fail "next: a preview writes nothing" "$(cat "$file")"; fi
    _look_run "/config appearance density roomy" "comfortable, compact" \
      "next: a bad value says what is accepted"
    _look_run "/config appearance permissionMode bypass" "not an appearance setting" \
      "next: only appearance settings can be changed"
    _look_run "/config appearance hints false" "Previewing hints = false" "next: hints preview off"
    _look_run "/config appearance save" "Saved." "next: the preview is saved"
    assert_eq "$(_look_json "$file")" \
      '{"hasCompletedOnboarding": true, '"$seen"', "nextAppearance": {"density": "compact", "glyphs": "unicode", "hints": false, "laterKey": 42}, "someFutureKey": {"kept": [1, 2]}}' \
      "next: a save changes only what it names"
  fi
  _look_quit

  # A file that cannot be read is refused and left alone.
  if _look_launch next preview "$scratch"; then
    _look_run "/config appearance layout inspect" "Previewing layout = inspect" \
      "next: a layout previews"
    assert_contains "$(tui_capture)" "Activity" "next: the inspector opens at once"
    cp "$file" "$scratch/good.json"
    printf '{ broken' >"$file"
    _look_run "/config appearance save" "Not saved" "next: an unreadable file refuses the save"
    assert_eq "$(cat "$file")" '{ broken' "next: the unreadable file is left as it was"
    cp "$scratch/good.json" "$file"
    _look_run "/config appearance revert" "Preview dropped" "next: a preview can be dropped"
    if wait_absent "canonical tasks" 2; then _pass "next: reverting keeps the session running"
    else _fail "next: reverting keeps the session running" "$(tui_capture)"; fi
  fi
  _look_quit

  # Project values stay out of the user's file, and the user hears who wins.
  printf '%s\n' '{"nextAppearance":{"theme":"light","colorMode":"16"}}' >"$project_file"
  if _look_launch next preview "$scratch"; then
    _look_run "/config appearance theme dark" "Previewing theme = dark" "next: a theme previews"
    _look_run "/config appearance save" "and those win here" \
      "next: a save says when this folder overrides it"
    assert_eq "$(_look_json "$file")" \
      '{"hasCompletedOnboarding": true, '"$seen"', "nextAppearance": {"density": "compact", "glyphs": "unicode", "hints": false, "laterKey": 42, "theme": "dark"}, "someFutureKey": {"kept": [1, 2]}}' \
      "next: project values stay out of the user's file"
  fi
  _look_quit
  printf '%s\n' '{}' >"$project_file"

  # Reset previews the defaults; only a save writes them.
  if _look_launch next preview "$scratch"; then
    cp "$file" "$scratch/before.json"
    _look_run "/config appearance reset" "Previewing the defaults" "next: a reset previews"
    if cmp -s "$file" "$scratch/before.json"; then _pass "next: a reset writes nothing yet"
    else _fail "next: a reset writes nothing yet" "$(cat "$file")"; fi
    _look_run "/config appearance save" "Saved." "next: a reset can be saved"
    assert_eq "$(_look_json "$file")" \
      '{"hasCompletedOnboarding": true, '"$seen"', "nextAppearance": {"laterKey": 42}, "someFutureKey": {"kept": [1, 2]}}' \
      "next: a reset keeps what this version does not know"
  fi
  _look_quit

  # The layout setting takes effect at launch, when the inspector fits.
  printf '{"hasCompletedOnboarding":true,%s,"nextAppearance":{"layout":"work"}}\n' "$seen" >"$file"
  if _look_launch next preview "$scratch"; then
    if wait_for "canonical tasks" 5; then _pass "next: the work layout opens the Work page"
    else _fail "next: the work layout opens the Work page" "$(tui_capture)"; fi
  fi
  _look_quit
  if TUI_WIDTH=100 _look_launch next preview "$scratch"; then
    assert_absent "$(tui_capture)" "canonical tasks" \
      "next: a narrow terminal keeps the transcript at launch"
  fi
  _look_quit

  # /settings changes reduced motion, and Next draws it without a restart.
  printf '{"hasCompletedOnboarding":true,%s,"nextAppearance":{"glyphs":"unicode"}}\n' "$seen" >"$file"
  if _look_launch next preview "$scratch"; then
    _look_turn
    if _look_wait_moving; then
      _pass "next: the indicator moves before the change"
    else _fail "next: the indicator moves before the change" "$(tui_capture)"; fi
    wait_absent "working" 12 || true
    tui_type "/settings"; sleep 0.3; tui_keys Enter
    if wait_for "Type to search settings" 8; then
      tui_type "reduce"; sleep 0.5; tui_keys Enter; sleep 0.5
      tui_keys Escape; sleep 0.3; tui_keys Escape; sleep 0.5
    fi
    assert_matches "$(_look_json "$file")" '"reduceMotion": true' "next: /settings saved the change"
    _look_turn
    if wait_for "• working" 8; then _pass "next: reduced motion applies without a restart"
    else _fail "next: reduced motion applies without a restart" "$(tui_capture)"; fi
    wait_absent "working" 12 || true
  fi
  _look_quit

  # Classic: the same command writes nothing, and the layout does not apply.
  printf '{"hasCompletedOnboarding":true,%s,"nextAppearance":{"layout":"work"}}\n' "$seen" >"$file"
  cp "$file" "$scratch/before.json"
  if _look_launch classic "Coven v" "$scratch"; then
    assert_absent "$(tui_capture)" "canonical tasks" "classic: the layout setting does nothing"
    tui_type "/config appearance density compact"; sleep 0.3; tui_keys Enter; sleep 1
    assert_absent "$(tui_capture)" "Previewing" "classic: nothing previews"
    tui_keys Escape; sleep 0.3
    tui_type "/config appearance save"; sleep 0.3; tui_keys Enter; sleep 1
    if cmp -s "$file" "$scratch/before.json"; then _pass "classic: the command writes nothing"
    else _fail "classic: the command writes nothing" "$(cat "$file")"; fi
  fi
  _look_quit

  TUI_LAUNCH_PREFIX="$saved_prefix"; TUI_BOOT_STRING="$saved_boot"
  kill "$fake_pid" 2>/dev/null; wait "$fake_pid" 2>/dev/null
  rm -rf "$scratch"
}
