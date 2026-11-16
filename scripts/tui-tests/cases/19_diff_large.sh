#!/usr/bin/env bash
# shellcheck shell=bash
#
# Reviewing a large change with /diff (#411). A frame used to format every line
# of the selected file and compare every word of a changed line, so each key took
# seconds on a large diff or on one changed line in a minified file. Both
# renderers show the same viewer. No network or model: /diff runs entirely inside
# the TUI.
#
# The same viewer on a binary change and two renames (#411): the binary file is
# marked binary and none of its bytes reach the terminal, and each rename names
# its old and new paths, with the edit of an edited one.

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

# A binary file whose bytes would color and retitle a terminal if any reached
# it, a pure rename, and a rename with an edit, all staged by git.
_diff_kinds_repo() {
  local repo="$1" git_home="$2"
  local git_cmd=(git -C "$repo" -c user.name=suite -c user.email=suite@example.invalid)
  HOME="$git_home" git -C "$repo" init -q
  printf 'BINHEAD\000\001\033[31mRAWRED\000\n' >"$repo/blob.bin"
  seq 1 20 | sed 's/^/kept line /' >"$repo/old-name.txt"
  seq 1 20 | sed 's/^/moved line /' >"$repo/before-edit.txt"
  HOME="$git_home" "${git_cmd[@]}" add -A
  HOME="$git_home" "${git_cmd[@]}" commit -qm base
  printf 'BINHEAD\000\011\033]0;RAWTITLE\007\000\n' >"$repo/blob.bin"
  HOME="$git_home" "${git_cmd[@]}" mv old-name.txt new-name.txt
  HOME="$git_home" "${git_cmd[@]}" mv before-edit.txt after-edit.txt
  sed 's/^moved line 10$/moved line TEN/' "$repo/after-edit.txt" >"$repo/edited"
  mv "$repo/edited" "$repo/after-edit.txt"
}

# The file list's row for a file, found by the end of its path, which the list
# keeps when it shortens a path. The row starts with the collapse mark; the
# detail pane's header, on a row of its own, names the file too.
_diff_kinds_row() { # end of the path
  tui_capture | grep -F -- "▾ " | grep -F -- "$1" | head -1
}

_diff_kinds_journey() {
  local ui="$1" boot="$2" scratch="$3" s
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
    local title_before; title_before="$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_title}')"
    tui_type "/diff"; tui_keys Enter

    # Git sorts the files by path: the edited rename comes first.
    if wait_for "before-edit.txt → after-edit.txt" 20; then
      _pass "$ui: an edited rename names its old and new paths"
    else
      _fail "$ui: an edited rename names its old and new paths" "$(tui_capture)"
    fi
    s="$(tui_capture)"
    assert_contains "$s" "before-edit.txt → after-edit.txt  +1 -1" "$ui: and counts its lines"
    assert_contains "$s" "moved line TEN" "$ui: and shows its edit"
    assert_matches "$(_diff_kinds_row "edit.txt")" 'renamed *│' "$ui: the list marks it renamed"

    tui_keys Down
    if wait_for "Binary file — no diff available" "$_DIFF_LARGE_WAIT"; then
      _pass "$ui: a binary change says it has no text diff"
    else
      _fail "$ui: a binary change says it has no text diff" "$(tui_capture)"
    fi
    s="$(tui_capture)"
    assert_contains "$(_diff_kinds_row "blob.bin")" "binary" "$ui: the list marks the file binary"
    assert_absent "$s" "BINHEAD" "$ui: none of the binary's text is drawn"
    assert_absent "$s" "RAWRED" "$ui: none of the binary's text after an escape is drawn"
    assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_title}')" "$title_before" \
      "$ui: the binary's title sequence never reached the terminal"

    tui_keys Down
    if wait_for "old-name.txt → new-name.txt" "$_DIFF_LARGE_WAIT"; then
      _pass "$ui: a pure rename names its old and new paths"
    else
      _fail "$ui: a pure rename names its old and new paths" "$(tui_capture)"
    fi
    assert_contains "$(tui_capture)" "Renamed from old-name.txt" "$ui: and says the content is unchanged"
    assert_matches "$(_diff_kinds_row "new-name.txt")" 'renamed *│' "$ui: the list marks it renamed, not +0 -0"

    tui_keys Escape
    if wait_absent "blob.bin" "$_DIFF_LARGE_WAIT"; then _pass "$ui: Esc closes the viewer"
    else _fail "$ui: Esc closes the viewer" "$(tui_capture)"; fi
    tui_keys C-c; sleep 0.3; tui_keys C-c
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
}

tc_diff_large() {
  describe "/diff on large, binary and renamed changes"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/repo"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  _diff_large_repo "$scratch/repo" "$scratch/home"

  _diff_large_journey classic "Coven v" "$scratch"
  _diff_large_journey next "preview" "$scratch"

  rm -rf "$scratch/repo"
  mkdir -p "$scratch/repo"
  _diff_kinds_repo "$scratch/repo" "$scratch/home"
  _diff_kinds_journey classic "Coven v" "$scratch"
  _diff_kinds_journey next "preview" "$scratch"

  rm -rf "$scratch"
}
