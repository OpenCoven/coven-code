#!/usr/bin/env bash
# shellcheck shell=bash
#
# Reviewing a large change with /diff (#411). A frame used to format every line
# of the selected file and compare every word of a changed line, so each key took
# seconds on a large diff or on one changed line in a minified file. Both
# renderers show the same viewer. No network or model: /diff runs entirely inside
# the TUI.

register_case tc_diff_large

# Short enough that a viewer formatting the whole file cannot pass, and long
# enough for a debug build on a busy runner that formats only what is on screen.
_DIFF_LARGE_WAIT=6

_diff_large_repo() {
  local repo="$1" git_home="$2"
  local git_cmd=(git -C "$repo" -c user.name=suite -c user.email=suite@example.invalid)
  HOME="$git_home" git -C "$repo" init -q
  seq 1 150000 | sed 's/^/old line /' >"$repo/huge.txt"
  seq 1 8000 | sed 's/^/old/' | tr '\n' ' ' >"$repo/min.js"
  HOME="$git_home" "${git_cmd[@]}" add -A
  HOME="$git_home" "${git_cmd[@]}" commit -qm base
  seq 1 150000 | sed 's/^/new line /' >"$repo/huge.txt"
  seq 1 8000 | sed 's/^/new/' | tr '\n' ' ' >"$repo/min.js"
}

_diff_large_journey() {
  local ui="$1" boot="$2" scratch="$3"
  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/repo") && env"
  local var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" "COVEN_CODE_UI=$ui")"
  TUI_BOOT_STRING="$boot"

  if tui_start; then
    tui_type "/diff"; tui_keys Enter
    if wait_for "old line 1" 20; then _pass "$ui: /diff opens on the large file"
    else _fail "$ui: /diff opens on the large file" "$(tui_capture)"; fi

    # Git lists every removal, then every addition, and the header takes row 0.
    # Twenty presses of three rows each put removed line 60 at the top of the pane.
    tui_keys Tab
    local press
    for press in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do tui_keys Down; done
    if wait_for "old line 60" "$_DIFF_LARGE_WAIT"; then _pass "$ui: scrolling a large file keeps up"
    else _fail "$ui: scrolling a large file keeps up" "$(tui_capture)"; fi

    # The next file is one enormous changed line.
    tui_keys Tab; tui_keys Down
    if wait_for "new1 new2" "$_DIFF_LARGE_WAIT"; then _pass "$ui: one enormous changed line opens"
    else _fail "$ui: one enormous changed line opens" "$(tui_capture)"; fi

    tui_keys Up
    if wait_for "old line 1" "$_DIFF_LARGE_WAIT"; then _pass "$ui: leaving that line keeps up"
    else _fail "$ui: leaving that line keeps up" "$(tui_capture)"; fi

    tui_keys Escape
    if wait_absent "huge.txt" "$_DIFF_LARGE_WAIT"; then _pass "$ui: Esc closes the viewer"
    else _fail "$ui: Esc closes the viewer" "$(tui_capture)"; fi
    tui_keys C-c; sleep 0.3; tui_keys C-c
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
}

tc_diff_large() {
  describe "Large diffs stay responsive in /diff"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/repo"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  _diff_large_repo "$scratch/repo" "$scratch/home"

  _diff_large_journey classic "Coven v" "$scratch"
  _diff_large_journey next "preview" "$scratch"

  rm -rf "$scratch"
}
