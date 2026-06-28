#!/usr/bin/env bash
# shellcheck shell=bash
#
# Headless CLI surface — no tmux, no TTY, no network. These exercise the
# argument parser and the offline subcommands that must work before release.

register_case tc_headless

tc_headless() {
  describe "Headless CLI surface (offline)"

  # --version prints the workspace version.
  run_bin --version
  assert_eq "$RUN_RC" "0" "--version exits 0"
  assert_contains "$RUN_OUT" "coven-code" "--version names the binary"
  # Version is a semver-ish x.y.z; assert the shape, not a hardcoded number.
  assert_matches "$RUN_OUT" '[0-9]+\.[0-9]+\.[0-9]+' "--version prints a semver"

  # --help describes usage and a couple of stable flags.
  run_bin --help
  assert_eq "$RUN_RC" "0" "--help exits 0"
  assert_contains "$RUN_OUT" "Usage: coven-code" "--help shows usage line"
  assert_contains "$RUN_OUT" "--print"            "--help lists --print"
  assert_contains "$RUN_OUT" "--model"            "--help lists --model"
  assert_contains "$RUN_OUT" "--permission-mode"  "--help lists --permission-mode"

  # An unknown flag must error and exit non-zero (clap behavior).
  run_bin --definitely-not-a-flag
  if [ "$RUN_RC" -ne 0 ]; then
    _pass "unknown flag exits non-zero (rc=$RUN_RC)"
  else
    _fail "unknown flag exits non-zero" "$RUN_OUT"
  fi

  # auth status: must run offline and report a status without crashing.
  # Outcome (logged in / not) depends on the machine, so accept either,
  # but require a clean exit and recognizable wording.
  run_bin auth status
  if [ "$RUN_RC" -eq 0 ] || [ "$RUN_RC" -eq 1 ]; then
    _pass "auth status exits cleanly (rc=$RUN_RC)"
  else
    _fail "auth status exits cleanly" "$RUN_OUT"
  fi
  assert_matches "$RUN_OUT" '[Ll]ogged in|[Nn]ot logged in|[Ll]og in' \
    "auth status reports a login state"

  # models: the static model catalog renders offline.
  run_bin models
  assert_eq "$RUN_RC" "0" "models exits 0"
  assert_matches "$RUN_OUT" 'ctx:.*in:.*out:' "models lists priced model rows"

  # --dump-system-prompt: offline prompt assembly (hidden flag).
  run_bin --dump-system-prompt
  assert_eq "$RUN_RC" "0" "--dump-system-prompt exits 0"
  assert_contains "$RUN_OUT" "Working directory" "system prompt includes working directory"
}
