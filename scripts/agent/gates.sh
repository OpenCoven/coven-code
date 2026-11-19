#!/usr/bin/env bash
# usage: scripts/agent/gates.sh [--dry-run] <worktree> <outdir>
#
# Runs the local gates that Rust CI runs, from <worktree>/src-rust:
#   fmt                  cargo fmt --all -- --check
#   check                cargo check --workspace --locked
#   clippy               cargo clippy --workspace --all-targets --locked -- -D warnings
#   clippy-computer-use  the same for claurst-tools with --features computer-use
#                        (macOS only, as in CI; its dependencies need extra
#                        libraries elsewhere)
#   test                 cargo test --workspace --locked --no-fail-fast
#   alloc                the next_frame_perf allocation gate, serial
# Each gate's output goes to <outdir>/<gate>.log. <outdir>/summary.txt gets one
# line per gate ("<gate> rc=<n> <seconds>s", or "<gate> skipped") and the
# workspace test counts;
# <outdir>/identity.txt records the head, tree and dirty files tested.
# --dry-run prints the commands and writes nothing.
#
# Environment (default):
#   COVEN_TOOLCHAIN   toolchain for `rustup run` (the dtolnay/rust-toolchain pin
#                     in <worktree>/.github/workflows/rust-ci.yml)
#   CARGO_TARGET_DIR  build directory (<worktree>/src-rust/target). Never share
#                     one between worktrees.
#   CARGO_BUILD_JOBS  passed to cargo when set; set it on a shared host
#   RUSTUP_HOME, CARGO_HOME  ($HOME/.rustup, $HOME/.cargo) kept for the test
#                     gates, which run with a scratch HOME under <outdir>
#   RUSTUP            rustup binary (rustup on PATH, else $CARGO_HOME/bin/rustup)
#   ONLY_TESTS=1      run only the test and alloc gates
#   GATES_COMPUTER_USE  1 or 0 to force the computer-use clippy gate on or off
#                     (on when uname -s is Darwin)
#
# Not run here: the macOS job's computer_use::tests and workspace_policy
# steps, the Windows job, the contract fixtures by name, and the Coven
# engine contract. Hosted CI runs those.
#
# Exit status: 0 when every gate passed, 1 when any failed, 2 on usage errors.
set -u

usage() { echo "usage: $0 [--dry-run] <worktree> <outdir>" >&2; exit 2; }

DRY_RUN=0
if [ "${1:-}" = "--dry-run" ]; then DRY_RUN=1; shift; fi
[ $# -eq 2 ] || usage
for arg in "$1" "$2"; do case "$arg" in -*) usage ;; esac; done

WORKTREE="$(cd "$1" 2>/dev/null && pwd -P)" || { echo "gates: no such worktree: $1" >&2; exit 2; }
[ -d "$WORKTREE/src-rust" ] || { echo "gates: $WORKTREE has no src-rust directory" >&2; exit 2; }
case "$2" in /*) OUT="$2" ;; *) OUT="$PWD/$2" ;; esac

TOOLCHAIN="${COVEN_TOOLCHAIN:-}"
if [ -z "$TOOLCHAIN" ]; then
  TOOLCHAIN="$(grep -m1 -oE 'dtolnay/rust-toolchain@[0-9][0-9.]*' \
    "$WORKTREE/.github/workflows/rust-ci.yml" 2>/dev/null | cut -d@ -f2)"
fi
[ -n "$TOOLCHAIN" ] || { echo "gates: no toolchain pin found in rust-ci.yml; set COVEN_TOOLCHAIN" >&2; exit 2; }

export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$WORKTREE/src-rust/target}"
# Colour codes would break the test-count summary.
export CARGO_TERM_COLOR=never
COMPUTER_USE="${GATES_COMPUTER_USE:-}"
if [ -z "$COMPUTER_USE" ]; then
  if [ "$(uname -s)" = Darwin ]; then COMPUTER_USE=1; else COMPUTER_USE=0; fi
fi
RUSTUP="${RUSTUP:-$(command -v rustup || echo "$CARGO_HOME/bin/rustup")}"
CARGO=("$RUSTUP" run "$TOOLCHAIN" cargo)

if [ "$DRY_RUN" = 1 ]; then
  SCRATCH_HOME="$OUT/home.XXXXXX"
else
  mkdir -p "$OUT" || exit 2
  SCRATCH_HOME="$(mktemp -d "$OUT/home.XXXXXX")" || exit 2
  trap 'rm -rf "$SCRATCH_HOME"' EXIT
  : >"$OUT/summary.txt"
  {
    echo "head $(git -C "$WORKTREE" rev-parse HEAD) tree $(git -C "$WORKTREE" rev-parse 'HEAD^{tree}')"
    echo "toolchain $TOOLCHAIN target $CARGO_TARGET_DIR jobs ${CARGO_BUILD_JOBS:-default}"
    git -C "$WORKTREE" status --porcelain
  } >"$OUT/identity.txt"
fi
TEST_ENV=(env HOME="$SCRATCH_HOME")

cd "$WORKTREE/src-rust" || exit 2
FAILED=0
run() {
  local name="$1" start rc
  shift
  if [ "$DRY_RUN" = 1 ]; then
    printf '%s:' "$name"
    printf ' %q' "$@"
    printf '\n'
    return
  fi
  start=$(date +%s)
  "$@" >"$OUT/$name.log" 2>&1
  rc=$?
  [ "$rc" -eq 0 ] || FAILED=1
  echo "$name rc=$rc $(($(date +%s) - start))s" | tee -a "$OUT/summary.txt"
}

if [ "$DRY_RUN" = 1 ]; then
  echo "cd $(printf '%q' "$PWD")"
  echo "toolchain=$TOOLCHAIN CARGO_TARGET_DIR=$CARGO_TARGET_DIR CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-}"
fi
if [ "${ONLY_TESTS:-}" != 1 ]; then
  run fmt "${CARGO[@]}" fmt --all -- --check
  run check "${CARGO[@]}" check --workspace --locked
  run clippy "${CARGO[@]}" clippy --workspace --all-targets --locked -- -D warnings
  if [ "$COMPUTER_USE" = 1 ]; then
    run clippy-computer-use "${CARGO[@]}" clippy --package claurst-tools --all-targets \
      --features computer-use --locked -- -D warnings
  elif [ "$DRY_RUN" = 1 ]; then
    echo "clippy-computer-use: skipped (not macOS; set GATES_COMPUTER_USE=1 to run it)"
  else
    echo "clippy-computer-use skipped" | tee -a "$OUT/summary.txt"
  fi
fi
run test "${TEST_ENV[@]}" "${CARGO[@]}" test --workspace --locked --no-fail-fast
run alloc "${TEST_ENV[@]}" "${CARGO[@]}" test -p claurst-tui --locked --test next_frame_perf \
  -- --ignored --skip measure_active_frames --test-threads=1
[ "$DRY_RUN" = 1 ] && exit 0

python3 -I - "$OUT/test.log" >>"$OUT/summary.txt" <<'PY'
import re
import sys

passed = failed = ignored = 0
failures = []
with open(sys.argv[1], errors="replace") as log:
    for line in log:
        m = re.search(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;", line)
        if m:
            passed += int(m[1])
            failed += int(m[2])
            ignored += int(m[3])
        if re.match(r"^test .* FAILED$", line.strip()):
            failures.append(line.strip())
print(f"tests passed={passed} failed={failed} ignored={ignored}")
for failure in failures:
    print("  ", failure)
PY
echo "result $([ "$FAILED" = 0 ] && echo pass || echo FAIL)" | tee -a "$OUT/summary.txt"
exit "$FAILED"
