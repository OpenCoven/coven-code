#!/usr/bin/env bash
# shellcheck shell=bash
#
# Next at 180x50 (#410): navigation is the Views page of the one inspector. It
# opens only when requested and the draft survives every step. That the
# conversation stays beside it at this size, a fresh session having none, is
# proven in crates/tui/tests/next_shell.rs.

register_case tc_next_wide_navigation

tc_next_wide_navigation() {
  describe "Next navigation on a wide terminal (COVEN_CODE_UI=next)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/acme-next"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  local saved_width="$TUI_WIDTH" saved_height="$TUI_HEIGHT"
  # Scrub anything that could reach a real provider or engine home.
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-next") && env"
  local var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next)"
  TUI_BOOT_STRING="preview"
  TUI_WIDTH=180 TUI_HEIGHT=50

  if tui_start; then
    local s
    tui_type "wide draft"
    wait_for "wide draft" 5 >/dev/null || true
    s="$(tui_capture)"
    assert_absent "$s" "local only" "navigation does not open by itself at 180x50"
    # Every inspector page has a "next page" footer, so no page opened at all.
    assert_absent "$s" "next page" "no inspector page opens by itself at 180x50"

    tui_keys M-i
    wait_for "Activity" 5 >/dev/null || true
    tui_keys BTab
    if wait_for "local only" 5; then _pass "navigation opens on request at 180x50"
    else _fail "navigation opens on request at 180x50" "$(tui_capture)"; fi
    s="$(tui_capture)"
    assert_contains "$s" "Ctrl+K" "navigation points to the existing palette"
    assert_contains "$s" "next page" "an open page shows the footer the closed check looks for"
    assert_contains "$s" "wide draft" "the draft stays beside navigation"
    assert_contains "$s" "acme-next" "the workspace line stays on screen"

    _tmux resize-window -t "$TUI_SESSION" -x 80 -y 24
    if wait_for "local only" 5; then _pass "navigation stays open at 80x24"
    else _fail "navigation stays open at 80x24" "$(tui_capture)"; fi
    assert_contains "$(tui_capture)" "wide draft" "the draft survives the resize to 80x24"

    _tmux resize-window -t "$TUI_SESSION" -x 180 -y 50
    tui_settle
    tui_keys Escape
    if wait_absent "local only" 5; then _pass "Escape closes navigation"
    else _fail "Escape closes navigation" "$(tui_capture)"; fi
    assert_contains "$(tui_capture)" "wide draft" "the draft is still there after closing"
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  TUI_WIDTH="$saved_width" TUI_HEIGHT="$saved_height"
  rm -rf "$scratch"
}
