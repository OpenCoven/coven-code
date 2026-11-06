#!/usr/bin/env bash
# shellcheck shell=bash
#
# Idle output (#417): with no input and nothing changing, Next stays off the
# wire apart from restating the cursor every few seconds. Classic is the
# control. It still writes on every tick, so a low rate from Next cannot come
# from a probe that sees no writes at all.
#
# Only the idle rate is judged, plus one key sent after the idle window: a
# Next that hung or died after its first frame writes nothing at all, which
# would otherwise pass. The key has the harness's 2 s budget. The resize
# probes are not judged, so a slow one on a busy runner is not a failure here.

register_case tc_next_idle_writes

# Print the idle rate, or the harness's failures when it has none.
_idle_rate() {
  python3 -B -c 'import json, sys
result = json.load(open(sys.argv[1]))["results"][0]
rate = result["idle_writes_per_second_p50"]
print(rate if rate is not None else "no rate: " + "; ".join(result["failures"]))' "$1"
}

# Print "yes" if the key sent after the idle window was painted, or the
# harness's failures when it was not.
_answered_key() {
  python3 -B -c 'import json, sys
result = json.load(open(sys.argv[1]))["results"][0]
print("yes" if result["input_to_paint"]["n"] >= 1 else "no key echo: " + "; ".join(result["failures"]))' "$1"
}

tc_next_idle_writes() {
  describe "Next stays off the wire on an idle terminal"
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local harness="${BASH_SOURCE[0]%/*}/../perf_baseline.py"
  local scratch ui
  scratch="$(mktemp -d)"
  for ui in next classic; do
    python3 -B "$harness" --bin "$COVEN_BIN" --ui "$ui" --runs 1 --keys 1 \
      --size 80x24 --json "$scratch/$ui.json" >/dev/null 2>"$scratch/$ui.err" || true
    if [ ! -s "$scratch/$ui.json" ]; then
      _fail "the $ui idle probe ran" "$(cat "$scratch/$ui.err")"
      rm -rf "$scratch"
      return 0
    fi
  done

  local next_rate classic_rate next_alive
  next_rate="$(_idle_rate "$scratch/next.json")"
  classic_rate="$(_idle_rate "$scratch/classic.json")"
  next_alive="$(_answered_key "$scratch/next.json")"
  rm -rf "$scratch"

  if [ "$next_alive" = yes ]; then
    _pass "Next still paints a key after the idle window"
  else
    _fail "Next still paints a key after the idle window" "$next_alive"
  fi

  if python3 -B -c 'import sys; sys.exit(0 if float(sys.argv[1]) < 1 else 1)' "$next_rate" 2>/dev/null; then
    _pass "idle Next writes under once a second"
  else
    _fail "idle Next writes under once a second" "$next_rate"
  fi
  if python3 -B -c 'import sys; sys.exit(0 if float(sys.argv[1]) > 1 else 1)' "$classic_rate" 2>/dev/null; then
    _pass "the probe sees Classic's idle writes"
  else
    _fail "the probe sees Classic's idle writes" "$classic_rate"
  fi
}
