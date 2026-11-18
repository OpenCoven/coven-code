#!/usr/bin/env bash
# shellcheck shell=bash
#
# The saved permission mode at startup (#515), in Classic and Next, each in
# a throwaway home and project. `/config set permission-mode` saves a mode;
# the next start without `--permission-mode` must be in it and say so. A saved
# bypass mode is never restored, `--permission-mode` wins over anything saved,
# and a repository's `.coven-code/settings.json` cannot choose the mode.
# `/config get permission-mode` reports the mode the session is really in.
# No turn is sent, so there is no model, provider or network.

register_case tc_saved_permission_mode

# _saved_mode_get <mode as /config get prints it> <label>
_saved_mode_get() {
  tui_type "/config get permission-mode"; tui_keys Enter
  if wait_for "permission-mode = $1" 10; then _pass "$2"
  else _fail "$2" "$(tui_capture)"; fi
}

# _saved_mode_dismiss <label>: close the startup notification, which in an
# 80-column Classic frame covers the row `/config get` answers on.
_saved_mode_dismiss() {
  tui_keys Escape
  if wait_absent "restored" 10; then _pass "$1"
  else _fail "$1" "$(tui_capture)"; fi
}

# _saved_mode_start [binary args...]: tui_start, which records a failed boot
# itself; a session that did not boot is torn down so the next one can start.
_saved_mode_start() {
  tui_start "$@" && return 0
  tui_stop
  return 1
}

_saved_mode_renderer() {
  local ui="$1" name="$2" boot="$3"
  local scratch; scratch="$(mktemp -d)"
  local settings="$scratch/home/.coven-code/settings.json"
  mkdir -p "$scratch/home/.coven-code" "$scratch/project" "$scratch/repository/.coven-code"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$settings"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  local env_part=" env" var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_HOME COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT \
    COVEN_DAEMON_SOCKET COVEN_CODE_UI COVEN_CODE_CLAUDE_BIN; do
    env_part+=" -u $var"
  done
  # A standalone install: the engine home is ~/.coven-code under this HOME.
  env_part+=" $(printf '%q ' "HOME=$scratch/home")"
  [ -n "$ui" ] && env_part+="COVEN_CODE_UI=$ui "
  local in_project in_repository
  in_project="cd $(printf '%q' "$scratch/project") &&$env_part"
  in_repository="cd $(printf '%q' "$scratch/repository") &&$env_part"
  TUI_LAUNCH_PREFIX="$in_project"
  TUI_BOOT_STRING="$boot"

  # Nothing saved: default, and nothing to say about it.
  if _saved_mode_start; then
    _saved_mode_get Default "$name: with nothing saved the session is in default"
    assert_absent "$(tui_capture)" "restored" "$name: nothing is announced when nothing is saved"
    # Save a mode the way a user does.
    tui_type "/config set permission-mode accept-edits"; tui_keys Enter
    _saved_mode_get AcceptEdits "$name: /config set changes the running session"
    tui_stop
  fi
  if grep -q '"permission_mode": *"acceptEdits"' "$settings"; then
    _pass "$name: /config set saved acceptEdits to the settings file"
  else
    _fail "$name: /config set saved acceptEdits to the settings file" "$(cat "$settings")"
  fi

  # The restart the issue describes: no flag, so the saved mode applies.
  if _saved_mode_start; then
    assert_contains "$(tui_capture)" "Saved accept-edits mode restored" \
      "$name: the session says it restored accept-edits"
    _saved_mode_dismiss "$name: the notification can be dismissed"
    _saved_mode_get AcceptEdits "$name: a restart without the flag is in the saved mode"
    tui_stop
  fi

  # The flag wins over the saved mode, and then there is nothing to announce.
  if _saved_mode_start --permission-mode plan; then
    _saved_mode_get Plan "$name: --permission-mode plan wins over saved acceptEdits"
    assert_absent "$(tui_capture)" "restored" "$name: a flag start announces nothing"
    tui_stop
  fi
  if _saved_mode_start --permission-mode default; then
    _saved_mode_get Default "$name: --permission-mode default wins over saved acceptEdits"
    tui_stop
  fi

  # A saved bypass mode is never restored: default, said once, no bypass gate.
  printf '%s\n' '{"hasCompletedOnboarding": true, "config": {"permission_mode": "bypassPermissions"}}' >"$settings"
  if _saved_mode_start; then
    local shot; shot="$(tui_capture)"
    assert_contains "$shot" "Saved bypass mode not restored" "$name: the session says the saved bypass mode was not restored"
    assert_eq "$(grep -c "not restored" <<<"$shot")" "1" "$name: it says so once"
    _saved_mode_dismiss "$name: and that notification can be dismissed"
    _saved_mode_get Default "$name: a saved bypass mode starts in default"
    tui_stop
  fi

  # A repository's settings file cannot choose the mode.
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$settings"
  printf '%s\n' '{"config": {"permission_mode": "bypassPermissions"}}' \
    >"$scratch/repository/.coven-code/settings.json"
  TUI_LAUNCH_PREFIX="$in_repository"
  if _saved_mode_start; then
    _saved_mode_get Default "$name: a repository settings file cannot choose bypass"
    tui_stop
  fi
  printf '%s\n' '{"config": {"permission_mode": "acceptEdits"}}' \
    >"$scratch/repository/.coven-code/settings.json"
  if _saved_mode_start; then
    _saved_mode_get Default "$name: a repository settings file cannot choose acceptEdits"
    assert_absent "$(tui_capture)" "restored" "$name: and nothing claims a mode was restored"
    tui_stop
  fi
  # Nor can it reset the mode the user saved by leaving it out.
  printf '%s\n' '{"hasCompletedOnboarding": true, "config": {"permission_mode": "plan"}}' >"$settings"
  printf '%s\n' '{"config": {"model": "repository-model"}}' \
    >"$scratch/repository/.coven-code/settings.json"
  if _saved_mode_start; then
    assert_contains "$(tui_capture)" "Saved plan mode restored" \
      "$name: the session says it restored plan"
    _saved_mode_dismiss "$name: and the plan notification can be dismissed"
    _saved_mode_get Plan "$name: a repository settings file does not reset the saved mode"
    tui_stop
  fi

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

tc_saved_permission_mode() {
  describe "Saved permission mode at startup (Classic and Next)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  _saved_mode_renderer "" "Classic" "Coven v"
  _saved_mode_renderer "next" "Next" "preview"
}
