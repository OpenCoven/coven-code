#!/usr/bin/env bash
# shellcheck shell=bash
#
# Terminal restoration after a panic (#417). The product has no input that
# panics, so the tui crate's `panic_restore` example takes the terminal with
# the real setup_terminal and panics on request; panic_restore_pty.py runs it
# on a raw PTY. A main-thread panic must restore termios and every terminal
# mode before the message and exit 101; a worker-thread panic must leave the
# terminal alone. The probe is looked up next to COVEN_BIN, or set
# COVEN_PANIC_PROBE.

register_case tc_panic_restore

tc_panic_restore() {
  describe "Terminal restored after a panic"
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local probe="${COVEN_PANIC_PROBE:-${COVEN_BIN%/*}/examples/panic_restore}"
  if [ ! -x "$probe" ]; then
    _fail "panic probe is built" \
      "no $probe; build it with: (cd src-rust && cargo build -p claurst-tui --example panic_restore)"
    return 0
  fi

  local line out rc
  out="$(python3 -B "${BASH_SOURCE[0]%/*}/../panic_restore_pty.py" "$probe" 2>&1)"
  rc=$?
  while IFS= read -r line; do
    case "$line" in
      "ok "*) _pass "${line#ok }" ;;
      "FAIL "*) _fail "${line#FAIL }" ;;
    esac
  done <<<"$out"
  if ! grep -qE '^(ok|FAIL) ' <<<"$out"; then
    _fail "panic PTY driver reported checks" "$out"
  elif [ "$rc" -ne 0 ] && ! grep -q '^FAIL ' <<<"$out"; then
    _fail "panic PTY driver exited $rc" "$out"
  fi
}
