#!/usr/bin/env bash
# shellcheck shell=bash
#
# Terminal restoration after a panic (#417). The product has no input that
# panics, so the tui crate's `panic_restore` example takes the terminal with
# the real setup_terminal and panics on request; panic_restore_pty.py runs it
# on a raw PTY. A main-thread panic must restore termios and every terminal
# mode before the message and exit 101; a worker-thread panic must leave the
# terminal alone.
#
# The probe is built from source, so it is looked up in COVEN_BIN's cargo
# output (examples/panic_restore next to it), or set COVEN_PANIC_PROBE. When
# COVEN_BIN is a cargo build, a missing probe fails; for an installed binary
# there is nothing to check it against, so the case is skipped.

register_case tc_panic_restore

tc_panic_restore() {
  describe "Terminal restored after a panic"
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local dir="${COVEN_BIN%/*}" probe="${COVEN_PANIC_PROBE:-}"
  if [ -z "$probe" ]; then
    probe="$dir/examples/panic_restore"
    if [ ! -x "$probe" ] && [ ! -d "$dir/deps" ]; then
      _skip "COVEN_BIN is not a cargo build; set COVEN_PANIC_PROBE to check the source tree's probe"
      return 0
    fi
  fi
  if [ ! -x "$probe" ]; then
    local profile=""
    [ "${dir##*/}" = "release" ] && profile=" --release"
    _fail "panic probe is built" \
      "no $probe; build it with: (cd src-rust && cargo build$profile -p claurst -p claurst-tui --bins --example panic_restore)"
    return 0
  fi
  log "  probe: $probe"

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
