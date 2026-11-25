#!/usr/bin/env bash
# shellcheck shell=bash
#
# Exercise the same vim draft in Classic and Next (#569, #605). The search
# and command lines consume Backspace and Enter while the draft is nonempty.
# A local claude stand-in records any attempted turn before exiting.

register_case tc_vim_escape

_vim_wait_for() { # renderer, visible text
  if wait_for "$2" 10; then _pass "$1: shows '$2'"
  else _fail "$1: shows '$2'" "$(tui_capture)"; fi
}

_vim_escape_journey() { # renderer
  local r="$1" scratch
  scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/project"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"

  cat >"$scratch/claude" <<'FAKE'
#!/bin/sh
[ "$1" = "-p" ] || exit 0
echo turn >>"$HOME/turns"
IFS= read -r request || exit 1
printf '%s\n' "$request" >>"$HOME/requests"
printf '%s\n' '{"type":"assistant","message":{"model":"fake","content":[{"type":"text","text":"vim control received"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":3,"output_tokens":1}}'
FAKE
  chmod +x "$scratch/claude"
  : >"$scratch/home/turns"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" var ui=""
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/project") && env"
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN \
    COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  if [ "$r" = next ]; then ui="COVEN_CODE_UI=next"; TUI_BOOT_STRING="preview"
  else TUI_BOOT_STRING="Coven v"; fi
  # shellcheck disable=SC2086 # an empty selector adds nothing
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" "COVEN_CODE_CLAUDE_BIN=$scratch/claude" $ui)"

  if tui_start; then
    # With vim still off, an Escape takes any startup notice away and does
    # nothing else, so the Escapes below reach vim.
    tui_keys Escape; sleep 0.3
    tui_type "/vim"; sleep 0.3; tui_keys Enter
    _vim_wait_for "$r" "-- INSERT --"
    tui_type "hello"
    _vim_wait_for "$r" "hello"
    tui_keys Escape
    _vim_wait_for "$r" "-- NORMAL --"
    tui_type "0"; tui_type "i"
    _vim_wait_for "$r" "-- INSERT --"
    tui_type "say "
    tui_keys Escape
    _vim_wait_for "$r" "-- NORMAL --"
    tui_type "A"
    _vim_wait_for "$r" "-- INSERT --"
    tui_type " world"
    _vim_wait_for "$r" "say hello world"
    tui_keys Escape
    _vim_wait_for "$r" "-- NORMAL --"
    tui_type "v"
    _vim_wait_for "$r" "-- VISUAL --"
    tui_keys Escape
    _vim_wait_for "$r" "-- NORMAL --"

    tui_type "/hellox"
    _vim_wait_for "$r" "-- SEARCH --"
    tui_keys BSpace Enter
    _vim_wait_for "$r" "-- NORMAL --"
    # Insert at the search match and move the cursor to the end so a block
    # cursor cannot obscure the character beneath it in the pane capture.
    tui_type "i"
    _vim_wait_for "$r" "-- INSERT --"
    tui_type "a "
    tui_keys Escape
    _vim_wait_for "$r" "-- NORMAL --"
    tui_type "A"
    _vim_wait_for "$r" "say a hello world"
    tui_keys Escape
    _vim_wait_for "$r" "-- NORMAL --"

    tui_type ":qx"
    _vim_wait_for "$r" "-- COMMAND --"
    tui_keys BSpace
    tui_quit_and_check "$r: :q with a nonempty draft" Enter
    assert_eq "$(cat "$scratch/home/turns")" "" "$r: vim editing and :q sent no provider turn"
    assert_absent "$(tui_capture)" "panicked" "$r: vim editing and :q do not panic"
  fi
  tui_stop

  # The same provider fixture must observe a deliberate Insert-mode send.
  : >"$scratch/home/turns"
  : >"$scratch/home/requests"
  if tui_start; then
    tui_keys Escape; sleep 0.3
    tui_type "/vim"; sleep 0.3; tui_keys Enter
    _vim_wait_for "$r" "-- INSERT --"
    tui_type "ordinary insert submission"
    _vim_wait_for "$r" "ordinary insert submission"
    tui_keys Enter
    _vim_wait_for "$r" "vim control received"
    assert_eq "$(cat "$scratch/home/turns")" "turn" "$r: the control sent exactly one turn"
    assert_contains "$(cat "$scratch/home/requests")" '"text":"ordinary insert submission"' "$r: the provider received the control text"
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

tc_vim_escape() {
  describe "Vim modes, search and :q with a nonempty draft"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi
  _vim_escape_journey classic
  _vim_escape_journey next
}
