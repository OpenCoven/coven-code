"""Offline tests for gates.sh and pr-ready.sh: syntax, shellcheck, and the gates dry run."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
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
        base = {k: v for k, v in os.environ.items() if k not in ("COVEN_TOOLCHAIN", "CARGO_TARGET_DIR", "ONLY_TESTS")}
        base.update(RUSTUP="rustup", **env)
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
        result = self.dry_run(str(self.worktree), str(self.out), ONLY_TESTS="1")
        self.assertEqual(list(self.gate_lines(result.stdout)), ["test", "alloc"])

    def test_gates_match_rust_ci(self):
        ci = RUST_CI.read_text()
        result = self.dry_run(str(self.worktree), str(self.out))
        gates = self.gate_lines(result.stdout)
        for name in ("fmt", "check", "clippy", "clippy-computer-use", "alloc"):
            command = "cargo " + gates[name].split(" cargo ", 1)[1]
            self.assertIn(command, ci, name)

    def test_usage_errors_exit_2(self):
        self.assertEqual(self.dry_run().returncode, 2)
        self.assertEqual(self.dry_run(str(self.worktree)).returncode, 2)
        self.assertEqual(self.dry_run(str(self.out), str(self.out)).returncode, 2)
        (self.worktree / ".github" / "workflows" / "rust-ci.yml").write_text("no pin\n")
        result = self.dry_run(str(self.worktree), str(self.out))
        self.assertEqual(result.returncode, 2)
        self.assertIn("COVEN_TOOLCHAIN", result.stderr)


if __name__ == "__main__":
    unittest.main()
