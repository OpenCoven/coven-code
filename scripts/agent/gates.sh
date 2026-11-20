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
# <outdir>/identity.txt records the head, tree, a fingerprint of the changes
# and untracked files, and the dirty files tested.
# --dry-run prints the commands and writes nothing.
# <outdir> must be new, empty, or an earlier gates outdir (one holding
# identity.txt), since a run replaces the logs it names.
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
#   GATES_ONLY_TESTS=1  run only the test and alloc gates; the result line
#                     then names the gates skipped
#   GATES_COMPUTER_USE  1 or 0 to force the computer-use clippy gate on or off
#                     (on when uname -s is Darwin)
#
# Not run here: the headless-schemas job (release version preflight and the
# headless JSON Schema fixtures), the installer-checksums jobs, the macOS
# job's computer_use::tests and workspace_policy steps, the Windows job, the
# contract fixtures by name, and the Coven engine contract. Hosted CI runs
# those, so a local pass covers only the gates listed above.
#
# Exit status: 0 when every gate passed, 1 when any failed, 2 on usage errors,
# 3 when the gates passed but the worktree's head or dirty files changed
# during the run, so the pass does not describe one tree.
set -u

usage() { echo "usage: $0 [--dry-run] <worktree> <outdir>" >&2; exit 2; }

DRY_RUN=0
if [ "${1:-}" = "--dry-run" ]; then DRY_RUN=1; shift; fi
[ $# -eq 2 ] || usage
for arg in "$1" "$2"; do case "$arg" in -*) usage ;; esac; done

WORKTREE="$(cd "$1" 2>/dev/null && pwd -P)" || { echo "gates: no such worktree: $1" >&2; exit 2; }
[ -d "$WORKTREE/src-rust" ] || { echo "gates: $WORKTREE has no src-rust directory" >&2; exit 2; }
case "$2" in /*) OUT="$2" ;; *) OUT="$PWD/$2" ;; esac
# Resolve symlinks and .. in the existing part of the path.
OUT_PARENT="$(dirname "$OUT")"
if [ -d "$OUT_PARENT" ]; then
  OUT="$(cd "$OUT_PARENT" && pwd -P)/$(basename "$OUT")"
else
  case "/$OUT/" in */../*|*/./*) echo "gates: give <outdir> without . or .. when its parent does not exist" >&2; exit 2 ;; esac
fi
# Inside the worktree the logs would show up in identity.txt's git status.
case "$OUT/" in "$WORKTREE"/*) echo "gates: put <outdir> outside the worktree" >&2; exit 2 ;; esac
# A run removes and rewrites the logs it names; never do that to an unrelated
# directory that happens to hold a test.log.
if [ -e "$OUT" ] && [ ! -e "$OUT/identity.txt" ] && [ -n "$(ls -A "$OUT" 2>/dev/null || echo x)" ]; then
  echo "gates: <outdir> exists and is not empty or an earlier gates outdir; give a new one" >&2
  exit 2
fi

TOOLCHAIN="${COVEN_TOOLCHAIN:-}"
if [ -z "$TOOLCHAIN" ]; then
  TOOLCHAIN="$(grep -m1 -oE 'dtolnay/rust-toolchain@[0-9][0-9.]*' \
    "$WORKTREE/.github/workflows/rust-ci.yml" 2>/dev/null | cut -d@ -f2)"
fi
[ -n "$TOOLCHAIN" ] || { echo "gates: no toolchain pin found in rust-ci.yml; set COVEN_TOOLCHAIN" >&2; exit 2; }

export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$WORKTREE/src-rust/target}"
# CI sets none of these. A caller's RUSTFLAGS=--cap-lints=allow or
# CARGO_PROFILE_TEST_DEBUG_ASSERTIONS=false would let a gate pass that fails
# in CI, and a Coven agent shell's COVEN_HOME would point the tests at a real
# home instead of the scratch one. A CARGO_TARGET_<triple>_RUNNER of `true`
# would skip every test, and XDG_* would send test caches to the caller's real
# directories on Linux. RUSTC_BOOTSTRAP=1 would let #![feature] code build on
# the stable pin, and a workspace wrapper could rewrite every rustc call.
# Settings in $CARGO_HOME/config.toml, and in a .cargo/config.toml in any
# directory above the worktree, still apply. (COVEN_TOOLCHAIN was read above.)
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS RUSTDOCFLAGS CARGO_ENCODED_RUSTDOCFLAGS \
  CARGO_BUILD_RUSTFLAGS CARGO_BUILD_RUSTDOCFLAGS CARGO_BUILD_TARGET RUSTC CLIPPY_CONF_DIR \
  RUSTC_BOOTSTRAP RUSTC_WORKSPACE_WRAPPER CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER
for var in $(compgen -e); do
  case "$var" in
    CARGO_TARGET_*_RUSTFLAGS | CARGO_TARGET_*_RUNNER | CARGO_PROFILE_* | XDG_* | \
      COVEN_* | CLAURST_* | ANTHROPIC_* | OPENAI_* | CODEX_*)
      unset "$var" ;;
  esac
done
# Colour codes would break the test-count summary.
export CARGO_TERM_COLOR=never
COMPUTER_USE="${GATES_COMPUTER_USE:-}"
case "$COMPUTER_USE" in ""|0|1) ;; *) echo "gates: GATES_COMPUTER_USE must be 0 or 1" >&2; exit 2 ;; esac
if [ -z "$COMPUTER_USE" ]; then
  if [ "$(uname -s)" = Darwin ]; then COMPUTER_USE=1; else COMPUTER_USE=0; fi
fi
RUSTUP="${RUSTUP:-$(command -v rustup || echo "$CARGO_HOME/bin/rustup")}"
CARGO=("$RUSTUP" run "$TOOLCHAIN" cargo)

# The head, the tracked changes and the untracked files' contents, so an edit
# to an already dirty file during the run is noticed too. Writes no objects.
fingerprint() {
  {
    git -C "$WORKTREE" rev-parse HEAD
    git -C "$WORKTREE" diff HEAD --binary --no-ext-diff --no-textconv
    # One file at a time: a path git cannot hash (a nested repository shows
    # up as "dir/") must not hide the files listed after it.
    git -C "$WORKTREE" ls-files -z -o --exclude-standard | while IFS= read -r -d '' path; do
      printf '%s ' "$path"
      git -C "$WORKTREE" hash-object -- "$path" 2>/dev/null || echo unhashable
    done
  } 2>&1 | cksum
}

if [ "$DRY_RUN" = 1 ]; then
  SCRATCH_HOME="$OUT/home.XXXXXX"
else
  # Without git the change check below would compare two identical errors.
  git -C "$WORKTREE" rev-parse --verify -q HEAD >/dev/null || {
    echo "gates: $WORKTREE is not a git worktree with a commit" >&2
    exit 2
  }
  "$RUSTUP" run "$TOOLCHAIN" cargo --version >/dev/null 2>&1 || {
    echo "gates: '$RUSTUP run $TOOLCHAIN cargo' does not run; install the toolchain or set RUSTUP" >&2
    exit 2
  }
  mkdir -p "$OUT" || exit 2
  SCRATCH_HOME="$(mktemp -d "$OUT/home.XXXXXX")" || exit 2
  trap 'rm -rf "$SCRATCH_HOME"' EXIT
  # Logs from an earlier run into the same outdir would look current.
  rm -f "$OUT"/fmt.log "$OUT"/check.log "$OUT"/clippy.log "$OUT"/clippy-computer-use.log \
    "$OUT"/test.log "$OUT"/alloc.log
  : >"$OUT/summary.txt"
  {
    echo "head $(git -C "$WORKTREE" rev-parse HEAD) tree $(git -C "$WORKTREE" rev-parse 'HEAD^{tree}')"
    echo "toolchain $TOOLCHAIN target $CARGO_TARGET_DIR jobs ${CARGO_BUILD_JOBS:-default}"
    IDENTITY="$(fingerprint)"
    echo "fingerprint $IDENTITY"
    git -C "$WORKTREE" status --porcelain --untracked-files=all
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
SKIPPED=""
if [ "${GATES_ONLY_TESTS:-}" = 1 ]; then
  SKIPPED="fmt check clippy clippy-computer-use"
  for gate in fmt check clippy clippy-computer-use; do
    if [ "$DRY_RUN" = 1 ]; then
      echo "$gate: skipped (GATES_ONLY_TESTS=1)"
    else
      echo "$gate skipped (GATES_ONLY_TESTS=1)" | tee -a "$OUT/summary.txt"
    fi
  done
else
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
    SKIPPED="clippy-computer-use"
  fi
fi
run test "${TEST_ENV[@]}" "${CARGO[@]}" test --workspace --locked --no-fail-fast
run alloc "${TEST_ENV[@]}" "${CARGO[@]}" test -p claurst-tui --locked --test next_frame_perf \
  -- --ignored --skip measure_active_frames --test-threads=1
[ "$DRY_RUN" = 1 ] && exit 0

# A workspace test gate that passes without running a single test has not
# tested anything (a cargo runner or config override); count it as failed.
python3 -I - "$OUT/test.log" >>"$OUT/summary.txt" <<'PY' || FAILED=1
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
if passed == 0:
    print("tests: no test passed, so the test gate is counted as failed")
    sys.exit(1)
PY
if [ "$FAILED" != 0 ]; then
  echo "result FAIL" | tee -a "$OUT/summary.txt"
  exit 1
fi
if [ "$(fingerprint)" != "$IDENTITY" ]; then
  echo "result CHANGED (the worktree's head or files changed during the run)" | tee -a "$OUT/summary.txt"
  exit 3
fi
echo "result pass${SKIPPED:+ (skipped: $SKIPPED)}" | tee -a "$OUT/summary.txt"
exit 0
