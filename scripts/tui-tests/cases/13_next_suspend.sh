#!/usr/bin/env bash
# shellcheck shell=bash
#
# Suspend and resume (#410): Ctrl+Z in Next hands the terminal back to a real
# job-control shell, and `fg` returns the same draft to a working screen.
# Without such a shell nothing stops and a notice says so.
#
# Stopping is judged by who owns the pane, not by the shell's job message,
# which differs between shells. Cooked mode is not checked here: a job-control
# shell restores its own terminal modes when a job stops, so that check would
# pass even if the engine left the terminal raw.

register_case tc_next_suspend
register_case tc_next_suspend_without_a_shell

_pane_format() {
  _tmux display-message -p -t "$TUI_SESSION" "$1"
}

_wait_for_stopped_job() {
  local waited=0
  while [ "$waited" -lt 50 ]; do
    if [ "$(_pane_format '#{alternate_on}')" = "0" ] &&
      [ "$(_pane_format '#{pane_current_command}')" != "coven-code" ]; then
      return 0
    fi
    sleep 0.1; waited=$((waited + 1))
  done
  return 1
}

tc_next_suspend() {
  describe "Next suspends to the shell and resumes (Ctrl+Z, fg)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/acme-next"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-next") && env"
  local var
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next)"
  TUI_BOOT_STRING="preview"

  if tui_start; then
    tui_type "draft one"
    if wait_for "draft not sent" 5; then _pass "a draft is in the composer"
    else _fail "a draft is in the composer" "$(tui_capture)"; fi
    local mouse_before
    mouse_before="$(_pane_format '#{mouse_any_flag}')"

    # ---- suspended: the shell owns a normal terminal ---------------------
    tui_keys C-z
    if _wait_for_stopped_job; then
      _pass "Ctrl+Z stops the job and returns to the shell"
      assert_eq "$(_pane_format '#{cursor_flag}')" "1" "suspending shows the cursor"
      assert_eq "$(_pane_format '#{mouse_any_flag}')" "0" "suspending turns mouse reporting off"
      tui_type 'echo "SHELL_$((40 + 2))"'; tui_keys Enter
      if wait_for "SHELL_42" 10; then _pass "the shell takes commands while stopped"
      else _fail "the shell takes commands while stopped" "$(tui_capture)"; fi

      # ---- resumed: same draft on a working screen -----------------------
      tui_type "fg"; tui_keys Enter
      if wait_for "draft one" 10; then _pass "fg brings back the draft"
      else _fail "fg brings back the draft" "$(tui_capture)"; fi
      assert_contains "$(tui_capture)" "preview" "fg repaints the whole shell"
      assert_absent "$(tui_capture)" "SHELL_42" "fg leaves no shell text on the screen"
      assert_eq "$(_pane_format '#{alternate_on}')" "1" "fg returns to the alternate screen"
      assert_eq "$(_pane_format '#{mouse_any_flag}')" "$mouse_before" "fg restores mouse reporting"
      tui_type " two"
      if wait_for "draft one two" 5; then _pass "keys reach the composer after fg"
      else _fail "keys reach the composer after fg" "$(tui_capture)"; fi

      # ---- suspending twice works the same -------------------------------
      tui_keys C-z
      if _wait_for_stopped_job; then
        tui_type "fg"; tui_keys Enter
        if wait_for "draft one two" 10; then _pass "a second suspend and resume works"
        else _fail "a second suspend and resume works" "$(tui_capture)"; fi

        # ---- quit still restores the terminal ----------------------------
        tui_keys C-c; sleep 0.3; tui_keys C-c
        local waited=0
        while [ "$waited" -lt 50 ] && [ "$(_pane_format '#{pane_current_command}')" = "coven-code" ]; do
          sleep 0.1; waited=$((waited + 1))
        done
        assert_eq "$(_pane_format '#{alternate_on}')" "0" \
          "quitting after a resume leaves the alternate screen"
        assert_eq "$(_pane_format '#{cursor_flag}')" "1" \
          "quitting after a resume restores the cursor"
      else
        _fail "a second suspend and resume works" "$(tui_capture)"
      fi
    else
      _fail "Ctrl+Z stops the job and returns to the shell" "$(tui_capture)"
    fi
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

tc_next_suspend_without_a_shell() {
  describe "Next keeps running when no shell could resume it"
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local outcome label detail
  while IFS=$'\t' read -r outcome label detail; do
    if [ "$outcome" = "pass" ]; then _pass "$label"; else _fail "$label" "$detail"; fi
  done < <(python3 -B - "$COVEN_BIN" "${BASH_SOURCE[0]%/*}/.." <<'PY'
import os
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, sys.argv[2])
import perf_baseline as perf


def report(ok, label, detail=""):
    print("\t".join(("pass" if ok else "fail", label, detail)), flush=True)


with tempfile.TemporaryDirectory() as root:
    home, project = os.path.join(root, "home"), os.path.join(root, "project")
    os.makedirs(os.path.join(home, ".coven-code"))
    os.makedirs(project)
    with open(os.path.join(home, ".coven-code", "settings.json"), "w", encoding="utf-8") as f:
        f.write('{"hasCompletedOnboarding": true}')
    # A new session with no controlling shell: the OS discards SIGTSTP.
    s = perf.Session(os.path.abspath(sys.argv[1]), 80, 24, home, project, "next")
    try:
        report(s.wait_for(perf.ready_needles("next", False), 30) is not None, "Next starts")
        s.drain(0.5)
        os.write(s.master, b"\x1a")
        output, end = b"", time.perf_counter() + 2
        while time.perf_counter() < end:
            data, at = s.read(end - time.perf_counter())
            if at is not None:
                output += data
        state = subprocess.run(["ps", "-o", "stat=", "-p", str(s.proc.pid)],
                               capture_output=True, text=True, check=False).stdout.strip()
        report(not state.startswith("T"), "the process keeps running", state)
        visible = perf.ESCAPES.sub(b"", output)
        report(b"job-control" in visible, "a notice says there is nothing to return to",
               repr(visible[-200:]))
        pop, leave = output.find(b"\x1b[<1u"), output.find(b"\x1b[?1049l")
        report(0 <= pop < leave, "keyboard flags are popped on the alternate screen",
               f"pop at {pop}, leave at {leave}")
        os.write(s.master, b"q")
        report(s.wait_for([b"q"], 5) is not None, "keys still reach the composer")
    finally:
        s.close()
PY
)
}
