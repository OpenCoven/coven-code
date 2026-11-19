"""Offline tests for gates.sh and pr-ready.sh: syntax, shellcheck, and the gates dry run."""
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import textwrap
import unittest

HERE = Path(__file__).resolve().parent
GATES = HERE / "gates.sh"
SCRIPTS = sorted(HERE.glob("*.sh"))
RUST_CI = HERE.parents[1] / ".github" / "workflows" / "rust-ci.yml"
GATE_NAMES = ["fmt", "check", "clippy", "clippy-computer-use", "test", "alloc"]


class ShellTests(unittest.TestCase):
    def test_bash_syntax(self):
        for script in SCRIPTS:
            result = subprocess.run(["bash", "-n", str(script)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, f"{script.name}: {result.stderr}")

    @unittest.skipUnless(
        shutil.which("shellcheck") or os.environ.get("REQUIRE_SHELLCHECK") == "1",
        "shellcheck is not installed",
    )
    def test_shellcheck_clean(self):
        result = subprocess.run(["shellcheck", *map(str, SCRIPTS)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


class GatesDryRunTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.worktree = Path(self.tmp.name).resolve() / "wt"
        (self.worktree / "src-rust").mkdir(parents=True)
        workflows = self.worktree / ".github" / "workflows"
        workflows.mkdir(parents=True)
        (workflows / "rust-ci.yml").write_text("      - uses: dtolnay/rust-toolchain@9.8.7\n")
        self.out = Path(self.tmp.name).resolve() / "out"

    def tearDown(self):
        self.tmp.cleanup()

    def dry_run(self, *args, **env):
        base = {
            k: v
            for k, v in os.environ.items()
            if k not in ("COVEN_TOOLCHAIN", "CARGO_TARGET_DIR", "GATES_ONLY_TESTS", "GATES_COMPUTER_USE")
        }
        base.update(RUSTUP="rustup", GATES_COMPUTER_USE="1")
        base.update(env)
        return subprocess.run(
            ["bash", str(GATES), "--dry-run", *args], env=base, capture_output=True, text=True
        )

    def gate_lines(self, stdout):
        return {
            line.split(":", 1)[0]: line.split(":", 1)[1].strip()
            for line in stdout.splitlines()
            if line.split(":", 1)[0] in GATE_NAMES
        }

    def test_prints_every_gate_with_the_pinned_toolchain_and_writes_nothing(self):
        result = self.dry_run(str(self.worktree), str(self.out))
        self.assertEqual(result.returncode, 0, result.stderr)
        gates = self.gate_lines(result.stdout)
        self.assertEqual(list(gates), GATE_NAMES)
        for command in gates.values():
            self.assertIn("rustup run 9.8.7 cargo", command)
        self.assertIn(f"CARGO_TARGET_DIR={self.worktree}/src-rust/target", result.stdout)
        self.assertIn(f"env HOME={self.out}/home.", gates["test"])
        self.assertIn("--no-fail-fast", gates["test"])
        self.assertFalse(self.out.exists())

    def test_environment_overrides_toolchain_and_target_dir(self):
        result = self.dry_run(
            str(self.worktree), str(self.out), COVEN_TOOLCHAIN="1.2.3", CARGO_TARGET_DIR="/elsewhere/t"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("rustup run 1.2.3 cargo", result.stdout)
        self.assertIn("CARGO_TARGET_DIR=/elsewhere/t", result.stdout)

    def test_only_tests_skips_fmt_check_and_clippy(self):
        result = self.dry_run(str(self.worktree), str(self.out), GATES_ONLY_TESTS="1")
        gates = self.gate_lines(result.stdout)
        run = [gate for gate, command in gates.items() if not command.startswith("skipped")]
        self.assertEqual(run, ["test", "alloc"])
        self.assertEqual(
            [gate for gate, command in gates.items() if command == "skipped (GATES_ONLY_TESTS=1)"],
            ["fmt", "check", "clippy", "clippy-computer-use"],
        )

    def test_computer_use_gate_can_be_skipped(self):
        result = self.dry_run(str(self.worktree), str(self.out), GATES_COMPUTER_USE="0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("clippy-computer-use: skipped", result.stdout)
        self.assertNotIn("--features computer-use", result.stdout)

    def test_gates_match_rust_ci(self):
        ci = RUST_CI.read_text()
        result = self.dry_run(str(self.worktree), str(self.out))
        gates = self.gate_lines(result.stdout)
        for name in ("fmt", "check", "clippy", "clippy-computer-use", "alloc"):
            command = "cargo " + gates[name].split(" cargo ", 1)[1]
            self.assertIn(command, ci, name)

    def test_default_toolchain_is_read_from_the_real_rust_ci(self):
        pin = re.search(r"dtolnay/rust-toolchain@([0-9][0-9.]*)", RUST_CI.read_text()).group(1)
        result = self.dry_run(str(HERE.parents[1]), str(self.out))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"toolchain={pin} ", result.stdout)

    def test_usage_errors_exit_2(self):
        self.assertEqual(self.dry_run().returncode, 2)
        self.assertEqual(self.dry_run(str(self.worktree)).returncode, 2)
        self.assertEqual(self.dry_run(str(self.out), str(self.out)).returncode, 2)
        self.assertEqual(self.dry_run(str(self.worktree), "-x").returncode, 2)
        self.assertEqual(self.dry_run(str(self.worktree), str(self.worktree / "gates")).returncode, 2)
        sneaky = self.worktree.parent / "elsewhere" / ".." / "wt" / "gates"
        (self.worktree.parent / "elsewhere").mkdir()
        self.assertEqual(self.dry_run(str(self.worktree), str(sneaky)).returncode, 2)
        self.assertEqual(self.dry_run(str(self.worktree), str(self.out), GATES_COMPUTER_USE="yes").returncode, 2)
        missing = self.worktree.parent / "missing" / ".." / "wt" / "gates"
        self.assertEqual(self.dry_run(str(self.worktree), str(missing)).returncode, 2)
        (self.worktree / ".github" / "workflows" / "rust-ci.yml").write_text("no pin\n")
        result = self.dry_run(str(self.worktree), str(self.out))
        self.assertEqual(result.returncode, 2)
        self.assertIn("COVEN_TOOLCHAIN", result.stderr)


FAKE_RUSTUP = textwrap.dedent(
    """\
    #!/bin/sh
    # Stands in for `rustup run <toolchain> cargo <subcommand> ...`.
    echo "$* HOME=$HOME RUSTFLAGS=${RUSTFLAGS-unset}${CARGO_TARGET_X_RUSTFLAGS-}" >> "$FAKE_CALLS"
    case "$4" in
      clippy) exit "${FAKE_CLIPPY_RC:-0}" ;;
      test)
        [ -n "${FAKE_TOUCH:-}" ] && echo x > "$FAKE_TOUCH"
        echo "test result: ok. 3 passed; 1 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.1s"
        echo "test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.1s"
        echo "test widgets::draws ... FAILED"
        exit "${FAKE_TEST_RC:-0}" ;;
    esac
    exit 0
    """
)


class GatesRunTests(unittest.TestCase):
    """A real run against a fake rustup: summary, exit status and cleanup."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name).resolve()
        self.worktree = root / "wt"
        (self.worktree / "src-rust").mkdir(parents=True)
        git = ["git", "-C", str(self.worktree), "-c", "user.name=t", "-c", "user.email=t@example.invalid"]
        subprocess.run([*git, "init", "-q"], check=True)
        (self.worktree / "src-rust" / "README").write_text("x\n")
        subprocess.run([*git, "add", "src-rust/README"], check=True)
        subprocess.run([*git, "commit", "-q", "-m", "init"], check=True)
        self.rustup = root / "rustup"
        self.rustup.write_text(FAKE_RUSTUP)
        self.rustup.chmod(0o755)
        self.calls = root / "calls.log"
        self.out = root / "out"

    def tearDown(self):
        self.tmp.cleanup()

    def gates(self, **env):
        base = dict(os.environ, RUSTUP=str(self.rustup), COVEN_TOOLCHAIN="9.9.9", FAKE_CALLS=str(self.calls))
        base.pop("GATES_ONLY_TESTS", None)
        base.update(GATES_COMPUTER_USE="1")
        base.update(env)
        return subprocess.run(
            ["bash", str(GATES), str(self.worktree), str(self.out)], env=base, capture_output=True, text=True
        )

    def summary(self):
        return (self.out / "summary.txt").read_text().splitlines()

    def test_failing_gate_sets_exit_1_and_summary_records_it(self):
        result = self.gates(FAKE_CLIPPY_RC="1", FAKE_TEST_RC="101")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        rcs = {line.split()[0]: line.split()[1] for line in self.summary() if " rc=" in line}
        self.assertEqual(
            rcs,
            {
                "fmt": "rc=0",
                "check": "rc=0",
                "clippy": "rc=1",
                "clippy-computer-use": "rc=1",
                "test": "rc=101",
                "alloc": "rc=101",  # the fake fails every cargo test
            },
        )
        self.assertIn("tests passed=7 failed=1 ignored=2", self.summary())
        self.assertIn("   test widgets::draws ... FAILED", self.summary())
        self.assertEqual(self.summary()[-1], "result FAIL")

    def test_passing_run_exits_0_uses_scratch_home_and_removes_it(self):
        result = self.gates()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.summary()[-1], "result pass")
        identity = (self.out / "identity.txt").read_text()
        self.assertRegex(identity, r"^head [0-9a-f]{40} tree [0-9a-f]{40}\n")
        self.assertIn("toolchain 9.9.9", identity)
        test_calls = [c for c in self.calls.read_text().splitlines() if " cargo test " in c]
        self.assertIn("run 9.9.9 cargo --version", self.calls.read_text().splitlines()[0])
        self.assertEqual(len(test_calls), 2)
        for call in test_calls:
            self.assertIn(f"HOME={self.out}/home.", call)
        self.assertEqual(list(self.out.glob("home.*")), [])

    def test_missing_toolchain_exits_2_before_writing(self):
        result = self.gates(RUSTUP=shutil.which("false") or "/usr/bin/false")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("install the toolchain", result.stderr)
        self.assertFalse(self.out.exists())

    def test_stale_logs_from_an_earlier_run_are_removed(self):
        self.out.mkdir()
        (self.out / "identity.txt").write_text("head old\n")
        (self.out / "fmt.log").write_text("old\n")
        result = self.gates(GATES_ONLY_TESTS="1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((self.out / "fmt.log").exists())
        self.assertTrue((self.out / "test.log").exists())

    def test_unrelated_nonempty_outdir_is_refused(self):
        self.out.mkdir()
        (self.out / "test.log").write_text("someone else's\n")
        result = self.gates()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("not empty", result.stderr)
        self.assertEqual((self.out / "test.log").read_text(), "someone else's\n")
        self.assertEqual(sorted(p.name for p in self.out.iterdir()), ["test.log"])
        self.out.joinpath("test.log").unlink()
        self.assertEqual(self.gates().returncode, 0)

    def test_untracked_nested_repo_does_not_hide_later_edits(self):
        subprocess.run(["git", "init", "-q", str(self.worktree / "aaa")], check=True)
        notes = self.worktree / "src-rust" / "notes.txt"
        notes.write_text("before\n")
        result = self.gates(FAKE_TOUCH=str(notes))
        self.assertEqual(result.returncode, 3, result.stdout + result.stderr)

    def test_caller_rustflags_are_not_passed_to_cargo(self):
        flags = "--cap-lints=allow"
        self.assertEqual(self.gates(RUSTFLAGS=flags, CARGO_TARGET_X_RUSTFLAGS=flags).returncode, 0)
        for call in self.calls.read_text().splitlines():
            self.assertTrue(call.endswith("RUSTFLAGS=unset"), call)

    def test_worktree_change_during_the_run_exits_3(self):
        result = self.gates(FAKE_TOUCH=str(self.worktree / "src-rust" / "new.rs"))
        self.assertEqual(result.returncode, 3, result.stdout + result.stderr)
        self.assertTrue(self.summary()[-1].startswith("result CHANGED"))

    def test_edit_to_an_already_dirty_or_untracked_file_exits_3(self):
        for name in ("README", "notes/new.txt"):
            path = self.worktree / "src-rust" / name
            path.parent.mkdir(exist_ok=True)
            path.write_text("dirty before the run\n")
            shutil.rmtree(self.out, ignore_errors=True)
            result = self.gates(FAKE_TOUCH=str(path))
            self.assertEqual(result.returncode, 3, name + result.stdout + result.stderr)
        shutil.rmtree(self.out)
        self.assertEqual(self.gates().returncode, 0)

    def test_only_tests_runs_two_gates(self):
        result = self.gates(GATES_ONLY_TESTS="1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([line.split()[0] for line in self.summary() if " rc=" in line], ["test", "alloc"])
        self.assertEqual(
            self.summary()[-1], "result pass (skipped: fmt check clippy clippy-computer-use)"
        )
        skipped = [line for line in self.summary()[:-1] if "skipped" in line]
        self.assertEqual(
            skipped, [f"{gate} skipped (GATES_ONLY_TESTS=1)" for gate in ("fmt", "check", "clippy", "clippy-computer-use")]
        )


if __name__ == "__main__":
    unittest.main()
