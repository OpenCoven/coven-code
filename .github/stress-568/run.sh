#!/usr/bin/env bash
# Throwaway (#568 evidence). Usage: run.sh linux|macos, from src-rust.
set -uo pipefail
BASE=0716acb5
S=$(cd "$(dirname "$0")" && pwd)
OUT=$PWD/stress-out; mkdir -p "$OUT/bin"
FILES="crates/mcp/src/oauth.rs crates/mcp/src/rmcp_endpoint_privacy_tests.rs crates/tui/tests/presentation_parity.rs crates/cli/tests/workspace_policy.rs"
use() { # use before|after
  if [ "$1" = before ]; then git checkout "$BASE" -- $FILES; else git checkout HEAD -- $FILES; fi
}
build() { # build <label> <cargo args...>; copies the test executable to bin/<label>
  local label=$1; shift
  local exe
  exe=$(cargo test --locked --no-run --message-format=json "$@" 2>"$OUT/build-$label.err" \
    | jq -r 'select(.reason=="compiler-artifact" and .profile.test==true and .executable!=null) | .executable' | tail -1)
  if [ -z "$exe" ]; then echo "BUILD FAILED $label"; tail -40 "$OUT/build-$label.err"; return 1; fi
  cp "$exe" "$OUT/bin/$label"; echo "built $label"
}
expect() { # expect <pass|fail> <label> <args...>: run once, print outcome and panic lines
  local want=$1 label=$2; shift 2
  local log="$OUT/run-$label-$(echo "$*" | tr -c 'a-zA-Z0-9' _ | cut -c1-60).log"
  HOME=$(mktemp -d) "$OUT/bin/$label" "$@" >"$log" 2>&1; local rc=$?
  local got=pass; [ $rc -ne 0 ] && got=fail
  echo "MUTATION-RESULT want=$want got=$got rc=$rc bin=$label args=$*"
  grep -E "panicked at|^test .* (ok|FAILED)$" "$log" | head -8
  grep -A3 "panicked at" "$log" | grep -v "panicked at\|RUST_BACKTRACE\|^--$" | head -6
}
loop() { # loop <label> <workers> <runs-per-worker> [args...]: whole-binary stress
  local label=$1 par=$2 n=$3; shift 3
  local fails="$OUT/loop-$label.txt"; : >"$fails"
  for w in $(seq 1 "$par"); do
    ( H=$(mktemp -d); for i in $(seq 1 "$n"); do
        HOME=$H "$OUT/bin/$label" "$@" >"$OUT/loop-$label-w$w.log" 2>&1 \
          || grep -E "^test .* FAILED$|panicked at" -A2 "$OUT/loop-$label-w$w.log" >>"$fails"
      done ) &
  done
  wait
  echo "LOOP $label runs=$((par*n)) failed-tests=$(grep -c ' FAILED$' "$fails")"
  grep ' FAILED$' "$fails" | sort | uniq -c
  grep -A2 "panicked at" "$fails" | head -30
}

if [ "$1" = linux ]; then
  for v in before after; do
    use $v
    build $v-mcp -p claurst-mcp --lib
    build $v-tui -p claurst-tui --test presentation_parity
    python3 "$S/mutate.py" A $v && build $v-mcp-A -p claurst-mcp --lib; use $v
    python3 "$S/mutate.py" B $v && build $v-mcp-B -p claurst-mcp --lib; use $v
    python3 "$S/mutate.py" P $v && build $v-tui-P -p claurst-tui --test presentation_parity; use $v
  done
  use after
  echo "== unprivileged port start: $(sysctl -n net.ipv4.ip_unprivileged_port_start 2>/dev/null)"
  echo "== mutations"
  R=oauth::plugin_session_tests::plugin_revocation_releases_pending_oauth_callback_listener
  F=oauth::tests::bind_callback_listener_reuses_redirect_port
  G=rmcp_backend::endpoint_privacy_tests::get_refusal_redacts_userinfo_query_and_fragment
  D=rmcp_backend::endpoint_privacy_tests::discovered_post_refusal_redacts_userinfo_query_and_fragment
  expect fail before-mcp-A --exact $R
  expect pass after-mcp-A --exact $R
  expect fail before-mcp-B --exact $R
  expect pass after-mcp-B --exact $R
  expect fail before-mcp-A --exact $G
  expect fail before-mcp-A --exact $D
  expect pass after-mcp --exact $G
  expect pass after-mcp --exact $D
  expect pass after-mcp-A --exact $F
  python3 -c 'import socket,time; s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1",14555)); s.listen(); time.sleep(120)' & HOLD=$!
  sleep 1
  expect fail before-mcp --exact $F
  expect pass after-mcp --exact $F
  kill $HOLD
  for t in next_draws_permission_requests_itself_unless_another_surface_is_open next_hands_every_input_owning_surface_to_classic; do
    expect fail before-tui-P --exact $t
    expect pass after-tui-P --exact $t
  done
  echo "== stress: 8 parallel copies of the claurst-mcp lib tests, 40 runs each, plus 4 parallel presentation_parity loops"
  for v in before after; do
    loop $v-tui 4 100 & T=$!
    loop $v-mcp 8 40
    wait $T
    uptime
  done
else
  P=stopping_a_scoped_run_prevents_a_delayed_background_write
  for v in before after; do
    use $v
    build $v-cli -p claurst --test workspace_policy
    python3 "$S/mutate.py" S $v && build $v-cli-S -p claurst --test workspace_policy; use $v
  done
  use after
  expect fail before-cli-S --exact $P
  expect fail after-cli-S --exact $P
  echo "== stress: workspace_policy binary, 3 parallel copies, 10 runs each"
  loop after-cli 3 10
fi
