#!/usr/bin/env bash
# shellcheck shell=bash
#
# Next Memory page, offline (#415): canonical memory files exist on disk in a
# disposable COVEN_HOME, but no Coven daemon is running. The page must say
# the daemon is unreachable and read nothing from the files, so their
# synthetic canary, title and file name never reach the terminal. Closing
# the page must leave nothing behind and submit nothing.
#
# This proves the offline, no-filesystem-fallback path only. Reads against a
# live daemon are covered by the opt-in core test
# `live_daemon_memory_reads_withhold_unclassified_text`.

register_case tc_next_memory

tc_next_memory() {
  describe "Next Memory page offline (COVEN_CODE_UI=next)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/home/.coven/memory/sage" "$scratch/acme-memory"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  printf '%s\n' 'CANARY-7f3a-private durable fact' \
    >"$scratch/home/.coven/memory/sage/canaryfile.md"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-memory") && env"
  local var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next)"
  TUI_BOOT_STRING="preview"

  if tui_start; then
    tui_keys M-i
    if wait_for "Activity" 5; then _pass "Alt+I opens the inspector"
    else _fail "Alt+I opens the inspector" "$(tui_capture)"; fi
    tui_keys Tab Tab Tab Tab
    if wait_for "no disk reads" 8; then
      _pass "with no daemon the Memory page says it is offline"
    else
      _fail "with no daemon the Memory page says it is offline" "$(tui_capture)"
    fi
    local s
    s="$(tui_capture)"
    assert_contains "$s" "read only" "the page says it is read only"
    assert_contains "$s" "Local instructions" "the page separates /memory local instructions"
    assert_absent "$s" "CANARY" "no memory content is read from disk"
    assert_absent "$s" "canaryfile" "no memory file name is read from disk"
    assert_absent "$s" "[familiar memory]" "no memory row is invented"
    tui_keys Escape
    tui_settle
    s="$(tui_capture)"
    assert_absent "$s" "no disk reads" "Escape closes the Memory page"
    assert_contains "$s" "ready" "nothing was submitted"
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}
