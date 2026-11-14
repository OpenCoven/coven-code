#!/usr/bin/env bash
# shellcheck shell=bash
#
# Zero and tiny terminal sizes (#424): Classic starts at 0x0, 0x40 and 120x0,
# survives resizes to 120x0 (with the first Ctrl+C's toast), 0x40, 0x0, 1x1
# and 20x6, draws a full frame again at 120x40, exits 0, and restores the
# terminal. tmux cannot make a zero-sized window, so zero_size_pty.py drives a
# raw PTY instead.

register_case tc_classic_zero_size

tc_classic_zero_size() {
  describe "Classic at zero and tiny sizes"
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local line out rc
  out="$(python3 -B "${BASH_SOURCE[0]%/*}/../zero_size_pty.py" "$COVEN_BIN" 2>&1)"
  rc=$?
  while IFS= read -r line; do
    case "$line" in
      "ok "*) _pass "${line#ok }" ;;
      "FAIL "*) _fail "${line#FAIL }" ;;
    esac
  done <<<"$out"
  if ! grep -qE '^(ok|FAIL) ' <<<"$out"; then
    _fail "zero-size PTY driver reported checks" "$out"
  elif [ "$rc" -ne 0 ] && ! grep -q '^FAIL ' <<<"$out"; then
    _fail "zero-size PTY driver exited $rc" "$out"
  fi
}
