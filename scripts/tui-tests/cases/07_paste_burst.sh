#!/usr/bin/env bash
# shellcheck shell=bash
#
# Paste burst: a whole line delivered in one write must still submit.
#
# try_detect_paste_burst drains the event queue into a single paste whenever a
# character arrives with more input already behind it. It used to append the
# terminating Enter to that buffer as a literal '\n', so the keystroke that
# submits the line was consumed as text: the message sat in the composer with a
# trailing newline and was never sent. Enter looked dead and the conversation
# could not advance past its first turn.
#
# That shape is not exotic — it is how a host application drives an embedded
# pane (one write of `text + "\r"`) and how a terminal without bracketed paste
# delivers a clipboard paste. Typing by hand escapes it, which is why the bug
# hid for so long.
#
# `/help` is used as the payload because it is handled entirely inside the TUI:
# no network, no credentials, no model call.

register_case tc_paste_burst

tc_paste_burst() {
  describe "Paste burst preserves Enter"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  # ---- 1. A line plus its Enter, delivered in one write, must submit -------
  tui_paste "/help"$'\r'
  # The overlay is tall; poll for a late-rendered item so the assertion does
  # not race the draw (same guard as the help-overlay case).
  if wait_for "/permissions"; then
    _pass "burst-delivered line submits (help overlay opened)"
    local s; s="$(tui_capture)"
    assert_contains "$s" "Toggle help" "burst submit reached the slash-command path"
  else
    _fail "burst-delivered line submits (help overlay never opened)" "$(tui_capture)"
  fi

  tui_keys Escape
  wait_absent "Toggle help" 5

  # ---- 2. A multi-line burst with no trailing Enter must NOT submit --------
  # Interior newlines belong in the text: coalescing them is the whole reason
  # the burst detector exists (without it a pasted block arrives as several
  # separate messages). With no trailing Enter, nothing may be sent.
  local a="ALPHAqzx" b="BRAVOqzx"
  tui_paste "$a"$'\r'"$b"
  if wait_for "$b" 8; then
    local s2; s2="$(tui_capture)"
    # Anchoring the first line to the composer prompt is what makes this
    # discriminating: an unsent buffer renders as "> ALPHAqzx" on the prompt
    # row, whereas a submitted one moves into the transcript and the prompt
    # would carry the second line instead.
    assert_contains "$s2" "$TUI_PROMPT $a" "interior newline did not submit the first line"
    assert_contains "$s2" "$b"             "multi-line burst keeps the second line"
  else
    _fail "multi-line burst lands in the composer" "$(tui_capture)"
  fi

  tui_stop
}
