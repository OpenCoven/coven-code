#!/usr/bin/env bash
# shellcheck shell=bash
#
# Terminal restoration after a panic (#417). The product has no known input
# that panics, so the tui crate's `panic_restore` example takes the terminal
# with the real setup_terminal and panics on request; panic_restore_pty.py
# runs it on a raw PTY. A main-thread panic must restore termios and every
# terminal mode before the message and exit 101; a worker-thread panic must
# leave the terminal alone.
#
# The probe is built from source, so it is looked up in COVEN_BIN's cargo
# output (examples/panic_restore next to it), or set COVEN_PANIC_PROBE. When
# COVEN_BIN is a cargo build (cargo's dep-info, coven-code.d, is next to it),
# a missing probe fails, and so does one older than a file cargo built it
# from (a plain `cargo build` does not rebuild it); for an installed binary
# there is nothing to check it against, so the case is skipped. A missing
# COVEN_PANIC_PROBE always fails.

register_case tc_panic_restore

tc_panic_restore() {
  describe "Terminal restored after a panic"
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local dir="${COVEN_BIN%/*}" probe="${COVEN_PANIC_PROBE:-}"
  if [ -z "$probe" ]; then
    probe="$dir/examples/panic_restore"
    if [ ! -x "$probe" ] && [ ! -f "$COVEN_BIN.d" ]; then
      _skip "COVEN_BIN is not a cargo build; set COVEN_PANIC_PROBE to check the source tree's probe"
      return 0
    fi
  fi
  local profile="" build
  [ "${dir##*/}" = "release" ] && profile=" --release"
  build="(cd \"$ROOT/src-rust\" && cargo build$profile -p claurst -p claurst-tui --bins --example panic_restore)"
  if [ ! -x "$probe" ]; then
    _fail "panic probe is built" "no $probe; build it alongside COVEN_BIN (same tree, profile and target), for example: $build"
    return 0
  fi
  # Compare the probe with the files cargo built it from, as listed in its
  # dep-info: one line, "target: dep dep ...", with spaces in paths escaped as
  # "\ ". This catches a forgotten rebuild; it is not proof, since an edit
  # in the same second as the link, or while cargo was building, can slip
  # through. An explicit COVEN_PANIC_PROBE is used as given.
  if [ -z "${COVEN_PANIC_PROBE:-}" ] && [ -f "$probe.d" ]; then
    local deps="" dep us=$'\037'
    local -a files=()
    IFS= read -r deps <"$probe.d"
    deps="${deps#*: }"
    read -ra files <<<"${deps//\\ /$us}"
    for dep in ${files[@]+"${files[@]}"}; do
      dep="${dep//$us/ }"
      if [ "$dep" -nt "$probe" ]; then
        _fail "panic probe is current" "$dep is newer than $probe; rebuild it alongside COVEN_BIN (same tree, profile and target), for example: $build"
        return 0
      fi
    done
  fi
  log "  probe: $probe"

  local line out rc fails_before="$TESTS_FAIL"
  out="$(python3 -B "${BASH_SOURCE[0]%/*}/../panic_restore_pty.py" "$probe" 2>&1)"
  rc=$?
  while IFS= read -r line; do
    case "$line" in
      "ok "*) _pass "${line#ok }" ;;
      "FAIL "*" :: "*) line="${line#FAIL }"; _fail "${line%% :: *}" "${line#* :: }" ;;
      "FAIL "*) _fail "${line#FAIL }" ;;
    esac
  done <<<"$out"
  if ! grep -qE '^(ok|FAIL) ' <<<"$out"; then
    _fail "panic PTY driver reported checks" "$out"
  elif [ "$rc" -ne 0 ] && [ "$TESTS_FAIL" -eq "$fails_before" ]; then
    _fail "panic PTY driver exited $rc" "$out"
  fi
}
