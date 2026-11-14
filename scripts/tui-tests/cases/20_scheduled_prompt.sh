#!/usr/bin/env bash
# shellcheck shell=bash
#
# Scheduled prompts still fire in an interactive session. Headless runs no
# longer fire them (tests/engine_contract.rs), so this proves the scheduler
# still runs where it belongs. Waits up to 70 s for a minute boundary.

register_case tc_scheduled_prompt

tc_scheduled_prompt() {
  describe "An interactive session fires a due scheduled prompt"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  local engine_home="$scratch/home/.coven-code"
  mkdir -p "$engine_home" "$scratch/work"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$engine_home/settings.json"
  python3 -c 'import json, sys, time
json.dump([{"id": "due-fixture", "cron": "* * * * *", "prompt": "scheduled fixture prompt",
            "recurring": False, "durable": True, "created_at": int(time.time())}], open(sys.argv[1], "w"))' \
    "$engine_home/scheduled_tasks.json"

  # A stand-in for the keyless `claude` transport: records its argv, answers once.
  cat >"$scratch/claude" <<FAKE
#!/bin/sh
printf '%s\n' "\$@" > '$scratch/claude-argv'
cat > '$scratch/claude-stdin'
printf '%s\n' '{"type":"system","subtype":"init","session_id":"fake","model":"fake"}'
printf '%s\n' '{"type":"assistant","message":{"model":"fake","content":[{"type":"text","text":"ok"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":1,"output_tokens":1}}'
FAKE
  chmod 755 "$scratch/claude"

  local saved_prefix="$TUI_LAUNCH_PREFIX"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/work") && env"
  local var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_HOME COVEN_PARENT \
    COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$scratch/home" "COVEN_CODE_CLAUDE_BIN=$scratch/claude")"

  if tui_start; then
    # The scheduler wakes on the next whole minute; allow time for the turn.
    local waited=0
    while [ "$waited" -lt 70 ] && [ ! -s "$scratch/claude-stdin" ]; do
      sleep 1; waited=$((waited + 1))
    done
    if grep -qF "scheduled fixture prompt" "$scratch/claude-stdin" 2>/dev/null; then
      _pass "the due prompt ran in the interactive session"
    else
      _fail "the due prompt ran in the interactive session" "$(tui_capture)"
    fi
    if python3 -c 'import json, sys; sys.exit(0 if json.load(open(sys.argv[1])) == [] else 1)' \
      "$engine_home/scheduled_tasks.json" 2>/dev/null; then
      _pass "a fired one-shot prompt leaves the schedule"
    else
      _fail "a fired one-shot prompt leaves the schedule" "$(cat "$engine_home/scheduled_tasks.json" 2>&1)"
    fi
    if grep -qx -- "--permission-mode" "$scratch/claude-argv" 2>/dev/null; then
      _pass "the scheduled turn passes the session's permission mode"
    else
      _fail "the scheduled turn passes the session's permission mode" "$(cat "$scratch/claude-argv" 2>&1)"
    fi
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  rm -rf "$scratch"
}
