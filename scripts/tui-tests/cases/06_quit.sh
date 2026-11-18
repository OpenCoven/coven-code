#!/usr/bin/env bash
# shellcheck shell=bash
#
# Shutdown: Ctrl+C twice exits the TUI and returns the shell.
#
# Quitting must also give back the keyboard flags (#410). The kitty keyboard
# protocol keeps one flag stack per screen and the TUI pushes its flags on
# the alternate screen, so the pop has to be sent before leaving it. A pop
# sent afterwards lands on the main screen's stack: the alternate stack then
# keeps one entry per run, and the next full-screen program that sets no
# flags itself (`less`) gets every key as an escape code. tmux keeps a single
# stack, so quit_restore_pty.py reads this from the bytes on a raw PTY, in
# Classic and Next.

register_case tc_quit
register_case tc_quit_keyboard_flags

tc_quit() {
  describe "Quit / shutdown"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  # First Ctrl+C is "cancel"; the second confirms quit.
  tui_keys C-c; sleep 0.4
  tui_keys C-c

  # Prove the process actually exited by dropping a marker on the shell that
  # is revealed once the TUI tears down.
  if wait_for "Coven v" 2; then
    : # still showing — fall through to explicit check below
  fi
  tui_type "echo __TUI_EXITED__"; tui_keys Enter
  if wait_for "__TUI_EXITED__" 5; then
    _pass "Ctrl+C twice exits to the shell"
  else
    _fail "Ctrl+C twice exits to the shell" "$(tui_capture)"
  fi

  tui_stop
}

tc_quit_keyboard_flags() {
  describe "Quit pops the keyboard flags on the alternate screen"
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local line out rc fails_before="$TESTS_FAIL"
  out="$(python3 -B "${BASH_SOURCE[0]%/*}/../quit_restore_pty.py" "$COVEN_BIN" "$TUI_WIDTH" "$TUI_HEIGHT" 2>&1)"
  rc=$?
  while IFS= read -r line; do
    case "$line" in
      "ok "*) _pass "${line#ok }" ;;
      "FAIL "*" :: "*) line="${line#FAIL }"; _fail "${line%% :: *}" "${line#* :: }" ;;
      "FAIL "*) _fail "${line#FAIL }" ;;
    esac
  done <<<"$out"
  if ! grep -qE '^(ok|FAIL) ' <<<"$out"; then
    _fail "quit PTY driver reported checks" "$out"
  elif [ "$rc" -ne 0 ] && [ "$TESTS_FAIL" -eq "$fails_before" ]; then
    _fail "quit PTY driver exited $rc" "$out"
  fi
}
