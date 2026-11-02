#!/usr/bin/env bash
# shellcheck shell=bash
#
# Hostile model output at the real PTY boundary (#417), Next renderer.
#
# A fake `claude` answers with an OSC 52 clipboard write, an OSC 0 title,
# an OSC 8 hyperlink, a screen clear, cursor homing and C1 controls. tmux is
# set to accept clipboard writes, so a leaked OSC 52 would land in its paste
# buffers and a leaked OSC 0 would change the pane title. The prompt, which
# becomes the terminal title, carries the same sequences. No clipboard write
# may land, the title may show only the prompt, earlier screen content must
# survive, and the sequences must show as visible caret text. The buffer-level counterpart, covering every page
# and field, is tui/tests/next_hostile_output.rs.

register_case tc_next_hostile_output

_hostile_fake_claude() {
  cat >"$1" <<'FAKE'
#!/bin/sh
# Fake `claude`: one answer full of terminal control sequences.
cat >/dev/null &
printf '%s\n' '{"type":"system","subtype":"init","session_id":"fake","model":"fake-model"}'
printf '%s\n' '{"type":"assistant","message":{"model":"fake-model","content":[{"type":"text","text":"hostile-begin \u001b]52;c;cHduZWQ=\u0007 \u001b]0;PWNED-TITLE\u0007 \u001b]8;;https://evil.example\u001b\\link\u001b]8;;\u001b\\ \u001b[2J\u001b[H \u009b31m \u009d52;c;eA\u009c hostile-end"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":3,"output_tokens":1}}'
FAKE
  chmod +x "$1"
}

tc_next_hostile_output() {
  describe "Hostile output stays inert at the PTY (COVEN_CODE_UI=next)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/bin" "$scratch/acme-hostile"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  _hostile_fake_claude "$scratch/bin/claude"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-hostile") && env"
  local var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next \
    "COVEN_CODE_CLAUDE_BIN=$scratch/bin/claude")"
  TUI_BOOT_STRING="preview"

  # Let a leaked OSC 52 reach tmux's paste buffers, so leaking is observable.
  _tmux set-option -g set-clipboard on 2>/dev/null || true

  if tui_start; then
    local title_before buffers_before
    title_before="$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_title}')"
    buffers_before="$(_tmux list-buffers 2>/dev/null | wc -l | tr -d ' ')"

    # The prompt becomes the terminal title: paste one that tries to end the
    # title sequence early and write the clipboard and title itself.
    tui_paste "show me"$'\a\e]52;c;cHduZWQ=\a\e]0;PWNED-TITLE\a'
    tui_settle
    tui_keys Enter
    if wait_for "hostile-end" 15; then
      _pass "the hostile answer is rendered"
      tui_settle
      local s
      s="$(tui_capture)"
      assert_contains "$s" "acme-hostile" "a screen clear in output does not wipe the shell"
      assert_contains "$s" "show me" "earlier transcript content survives"
      assert_contains "$s" "^[]52;c;" "the clipboard write is shown as text"
      assert_contains "$s" "^[[2J" "the screen clear is shown as text"

      local title_after buffers_after
      title_after="$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_title}')"
      buffers_after="$(_tmux list-buffers 2>/dev/null | wc -l | tr -d ' ')"
      # The prompt becomes the title. Pasted controls already arrive in the
      # composer as caret text (^[), which is inert; a raw Escape or BEL must
      # never reach the title. Stored titles are covered by the unit test
      # the_terminal_title_topic_carries_no_control_characters_in_next.
      case "$title_after" in
        *$'\e'*|*$'\a'*)
          _fail "the title carries no control characters" "before=$title_before after=$title_after" ;;
        *"show me"*) _pass "the title carries no control characters" ;;
        *) _fail "the title carries no control characters" "before=$title_before after=$title_after" ;;
      esac
      if [ "$buffers_after" = "$buffers_before" ]; then _pass "no clipboard write reached tmux"
      else _fail "no clipboard write reached tmux" "$(_tmux list-buffers)"; fi
    else
      _fail "the hostile answer is rendered" "$(tui_capture)"
    fi
  fi
  tui_stop

  _tmux set-option -g set-clipboard external 2>/dev/null || true
  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}
