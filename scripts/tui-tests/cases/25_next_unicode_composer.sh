#!/usr/bin/env bash
# shellcheck shell=bash
#
# Committed Unicode text in the composer (#410): CJK, right-to-left text, a
# base letter with a combining mark and an emoji ZWJ sequence are typed through
# tmux and kept unsent, the text is edited, and all of it is sent to a local
# fake of the Messages API. The screen is checked for all but the emoji, whose
# drawing differs between tmux versions; the provider and the saved session
# must get exactly the text.
#
# Editing moves by grapheme cluster: one Backspace removes a whole ZWJ emoji or
# a letter with its combining mark, and Left steps over the emoji whole. Since
# the emoji cannot be read off the screen, that is proved by the text the
# provider gets: a cursor left inside a cluster would put the next typed
# letter inside it. The full journey runs in Next; Classic shares the composer
# and runs the cluster edits alone.
#
# tmux delivers committed characters only: an input method's preedit
# (composition) state cannot be driven through it, so that is not covered.

register_case tc_next_unicode_composer
register_case tc_classic_unicode_composer

# Whether a saved session or the provider's log holds exactly this text as a
# whole string value. Prints "yes" or "no".
_unicode_holds_exactly() { # text, files...
  python3 -B - "$@" <<'PY'
import json, sys
text, files = sys.argv[1], sys.argv[2:]

def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)

found = False
for path in files:
    try:
        with open(path, encoding="utf-8") as f:
            raw = f.read()
    except OSError:
        continue
    docs = [raw] if path.endswith(".json") else raw.splitlines()
    for doc in docs:
        try:
            if any(s == text for s in strings(json.loads(doc))):
                found = True
        except ValueError:
            pass
print("yes" if found else "no")
PY
}

# Start the fake provider and the TUI for `ui` ("next" or "classic") in a
# scratch home, run `journey` with the scratch directory, and clean up.
_unicode_run() { # ui, journey
  local ui="$1" journey="$2"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/acme-next" "$scratch/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  : >"$scratch/requests.log"

  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$scratch/port" "$scratch/requests.log" "$scratch/markers" 2>/dev/null &
  local fake_pid=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -s "$scratch/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-next") && env"
  local var
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$(cat "$scratch/port" 2>/dev/null)")"
  if [ "$ui" = next ]; then
    TUI_LAUNCH_PREFIX+="COVEN_CODE_UI=next "
    TUI_BOOT_STRING="preview"
  else
    TUI_BOOT_STRING="Coven v"
  fi

  # tui_start reports its own failure.
  if [ ! -s "$scratch/port" ]; then
    _fail "the fake provider starts" "no port file"
  elif tui_start; then
    "$journey" "$scratch"
  fi
  tui_stop
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

tc_next_unicode_composer() {
  describe "Next composer keeps committed Unicode text (COVEN_CODE_UI=next)"
  _unicode_run next _next_unicode_journey
}

tc_classic_unicode_composer() {
  describe "Classic composer edits by grapheme cluster (COVEN_CODE_UI unset)"
  _unicode_run classic _classic_unicode_journey
}

# Type one spare emoji and one spare letter-with-mark after a draft that ends
# in the emoji, remove both with one Backspace each, then step Left over the
# emoji and type Y in front of it. The draft then ends "Y<emoji>".
_unicode_edit_clusters() { # combining, zwj_emoji
  tui_type "$2$1"
  tui_keys BSpace BSpace
  tui_keys Left
  tui_type "Y"
  tui_keys Right
}

_classic_unicode_journey() {
  local scratch="$1"
  local combining zwj_emoji
  combining="$(printf 'e\314\201')"
  zwj_emoji="$(printf '\360\237\221\251\342\200\215\360\237\222\273')"
  local expected="${combining} Y${zwj_emoji} classic-done"

  tui_type "${combining} ${zwj_emoji}"
  _unicode_edit_clusters "$combining" "$zwj_emoji"
  tui_type " classic-done"
  if wait_for "classic-done" 5; then _pass "the edited draft is shown"
  else _fail "the edited draft is shown" "$(tui_capture)"; fi
  assert_contains "$(tui_capture)" "${combining} Y" "Y is typed in front of the emoji, after the letter and its mark"
  assert_eq "$(wc -l <"$scratch/requests.log" | tr -d ' ')" "0" "no turn reached the provider before Enter"

  tui_keys Enter
  local waited=0
  while [ "$waited" -lt 200 ] &&
    [ "$(_unicode_holds_exactly "$expected" "$scratch/requests.log")" != "yes" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  assert_eq "$(_unicode_holds_exactly "$expected" "$scratch/requests.log")" "yes" \
    "the provider got exactly the edited text"
}

_next_unicode_journey() {
  local scratch="$1" s
  # Built from bytes so no invisible character hides in this file:
  # e + U+0301 COMBINING ACUTE ACCENT, and U+1F469 U+200D U+1F4BB.
  local combining zwj_emoji
  combining="$(printf 'e\314\201')"
  zwj_emoji="$(printf '\360\237\221\251\342\200\215\360\237\222\273')"
  local expected="漢字Xか שלום مرحبا ${combining} Y${zwj_emoji} unicode-done"

  # ---- CJK is shown whole and stays unsent ------------------------------
  tui_type "漢字かな"
  if wait_for "漢字かな" 5; then _pass "CJK text is shown as typed"
  else _fail "CJK text is shown as typed" "$(tui_capture)"; fi
  assert_contains "$(tui_capture)" "draft not sent" "the CJK draft is unsent"

  # ---- moving and deleting act on whole characters ----------------------
  tui_keys Left Left
  tui_type "X"
  if wait_for "漢字Xかな" 5; then _pass "Left moves over whole CJK characters"
  else _fail "Left moves over whole CJK characters" "$(tui_capture)"; fi
  tui_keys Right Right BSpace
  if wait_absent "Xかな" 5; then
    s="$(tui_capture)"
    assert_contains "$s" "漢字Xか" "Backspace removes one whole CJK character"
  else
    _fail "Backspace removes one whole CJK character" "$(tui_capture)"
  fi

  # ---- right-to-left text, a combining mark and a ZWJ emoji -------------
  tui_type " שלום مرحبا"
  if wait_for "שלום" 5; then _pass "right-to-left text is shown"
  else _fail "right-to-left text is shown" "$(tui_capture)"; fi
  tui_type " ${combining} ${zwj_emoji}"
  # ---- a ZWJ emoji and a letter with its mark are each one character ----
  _unicode_edit_clusters "$combining" "$zwj_emoji"
  tui_type " unicode-done"
  if wait_for "unicode-done" 5; then _pass "the rest of the draft is shown"
  else _fail "the rest of the draft is shown" "$(tui_capture)"; fi
  s="$(tui_capture)"
  assert_contains "$s" "مرحبا" "Arabic text is shown"
  assert_contains "$s" "${combining} Y" "Y is typed in front of the emoji, after the letter and its mark"
  assert_contains "$s" "draft not sent" "the whole draft is still unsent"
  # The fake logs streamed requests, which is what a turn sends.
  assert_eq "$(wc -l <"$scratch/requests.log" | tr -d ' ')" "0" "no turn reached the provider before Enter"

  # ---- sent exactly, and saved exactly -----------------------------------
  tui_keys Enter
  # The fake answers with a Bash call that touches a marker named after the
  # prompt's last word; approving it lets the turn finish.
  if ! wait_for "Approve Bash" 20; then
    _fail "the prompt reaches the fake provider" "$(tui_capture)"
    return 0
  fi
  # A loaded host can drop a key pressed as the prompt opens: try up to 3 times.
  local tries=0
  while [ "$tries" -lt 3 ]; do
    sleep 0.7; tui_keys y
    if wait_absent "Approve Bash" 5; then break; fi
    tries=$((tries + 1))
  done
  if wait_for "TOOL_RESULT_OK" 30; then _pass "the turn finishes"
  else _fail "the turn finishes" "$(tui_capture)"; fi

  assert_eq "$(_unicode_holds_exactly "$expected" "$scratch/requests.log")" "yes" \
    "the provider got exactly the edited text"
  local sessions=() file waited=0
  while [ "$waited" -lt 50 ]; do
    sessions=()
    while IFS= read -r file; do sessions+=("$file"); done < <(
      find "$scratch/home" -type f -path '*/sessions/*.json' 2>/dev/null)
    if [ "${#sessions[@]}" -gt 0 ] &&
      [ "$(_unicode_holds_exactly "$expected" "${sessions[@]}")" = "yes" ]; then
      break
    fi
    sleep 0.1; waited=$((waited + 1))
  done
  if [ "${#sessions[@]}" -gt 0 ]; then
    assert_eq "$(_unicode_holds_exactly "$expected" "${sessions[@]}")" "yes" \
      "the saved session holds exactly the edited text"
  else
    _fail "the saved session holds exactly the edited text" "no session was saved"
  fi
}
