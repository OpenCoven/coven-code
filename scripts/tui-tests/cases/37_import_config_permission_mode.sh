#!/usr/bin/env bash
# shellcheck shell=bash
#
# `/config import` (formerly `/import-config`) reloads the saved settings into
# the open session (#578), in
# Classic and Next, each in a throwaway home and project. A saved bypass mode
# must not reach the session: it stays in default and says so once. A mode
# chosen at launch with `--permission-mode` survives the reload.
# `/config get permission-mode` reports the mode the session is really in.
# No turn is sent, so there is no model, provider or network.

register_case tc_import_config_permission_mode

# _import_mode_get <mode as /config get prints it> <label>
_import_mode_get() {
  tui_type "/config get permission-mode"; tui_keys Enter
  if wait_for "permission-mode = $1" 10; then _pass "$2"
  else _fail "$2" "$(tui_capture)"; fi
}

# _import_mode_run <label>: /config import, pick settings.json, confirm.
_import_mode_run() {
  tui_type "/config import"; tui_keys Enter
  if ! wait_for "settings.json" 10; then _fail "$1: the picker opens" "$(tui_capture)"; return 1; fi
  tui_keys Down; tui_settle 0.3; tui_keys Enter
  # Only the preview shows the target file; the picker shows sources.
  if ! wait_for "Target" 10; then _fail "$1: the preview opens" "$(tui_capture)"; return 1; fi
  tui_settle 0.3; tui_keys Enter
  tui_settle 1
  return 0
}

_import_mode_start() {
  tui_start "$@" && return 0
  tui_stop
  return 1
}

_import_mode_renderer() {
  local ui="$1" name="$2" boot="$3"
  local scratch; scratch="$(mktemp -d)"
  local settings="$scratch/home/.coven-code/settings.json"
  mkdir -p "$scratch/home/.coven-code" "$scratch/home/.claude" "$scratch/project"
  # The file /config import reads; it names no permission mode.
  printf '%s\n' '{"theme": "dark"}' >"$scratch/home/.claude/settings.json"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  local env_part=" env" var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_HOME COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT \
    COVEN_DAEMON_SOCKET COVEN_CODE_UI COVEN_CODE_CLAUDE_BIN; do
    env_part+=" -u $var"
  done
  env_part+=" $(printf '%q ' "HOME=$scratch/home")"
  [ -n "$ui" ] && env_part+="COVEN_CODE_UI=$ui "
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/project") &&$env_part"
  TUI_BOOT_STRING="$boot"

  # No flag: a bypass saved while the session runs is not applied by the
  # reload; the session stays in default and says so once.
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$settings"
  if _import_mode_start; then
    _import_mode_get Default "$name: the session starts in default"
    printf '%s\n' '{"hasCompletedOnboarding": true, "config": {"permission_mode": "bypassPermissions"}}' >"$settings"
    if _import_mode_run "$name"; then
      local shot; shot="$(tui_capture)"
      assert_contains "$shot" "Saved bypass mode not restored" \
        "$name: the reload says the saved bypass mode was not restored"
      assert_eq "$(grep -c "not restored" <<<"$shot")" "1" "$name: it says so once"
      tui_keys Escape
      if wait_absent "not restored" 10; then _pass "$name: the notification can be dismissed"
      else _fail "$name: the notification can be dismissed" "$(tui_capture)"; fi
      _import_mode_get Default "$name: the session is still in default after the reload"
    fi
    tui_stop
  fi

  # A launch flag survives the reload, whatever is saved.
  if _import_mode_start --permission-mode accept-edits; then
    _import_mode_get AcceptEdits "$name: --permission-mode accept-edits at launch"
    if _import_mode_run "$name (flag)"; then
      assert_absent "$(tui_capture)" "restored" "$name: a flag session announces nothing"
      _import_mode_get AcceptEdits "$name: the launch mode survives the reload"
    fi
    tui_stop
  fi

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

tc_import_config_permission_mode() {
  describe "Saved permission mode across /config import (Classic and Next)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  _import_mode_renderer "" "Classic" "Coven v"
  _import_mode_renderer "next" "Next" "preview"
}
