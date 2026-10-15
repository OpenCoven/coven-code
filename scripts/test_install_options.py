"""Check installer argument diagnostics without running an installation."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"


class InstallerOptionTests(unittest.TestCase):
    def run_installer(self, *args):
        with tempfile.TemporaryDirectory(prefix="coven-install-options-") as directory:
            result = subprocess.run(
                ["/bin/bash", str(INSTALLER), *args],
                env={**os.environ, "HOME": directory},
                text=True, capture_output=True, timeout=10,
            )
            self.assertEqual(list(Path(directory).iterdir()), [], "must not install")
            return result

    def test_missing_values_are_actionable(self):
        for option in ("--version", "--install-dir", "--binary"):
            with self.subTest(option=option):
                result = self.run_installer(option)
                self.assertEqual(result.returncode, 1)
                self.assertIn(f"Missing value for {option}", result.stderr)
                self.assertIn("USAGE:", result.stderr)
                self.assertNotIn("unbound variable", result.stderr)

    def test_values_are_consumed_before_help(self):
        for option in ("--version", "--install-dir", "--binary"):
            for value in ("value with spaces", ""):
                with self.subTest(option=option, value=value):
                    result = self.run_installer(option, value, "--help")
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("USAGE:", result.stdout)
                    self.assertEqual(result.stderr, "")

    def test_help_and_unknown_option(self):
        result = self.run_installer("--help")
        self.assertEqual(result.returncode, 0)
        self.assertIn("USAGE:", result.stdout)
        result = self.run_installer("--unknown")
        self.assertEqual(result.returncode, 1)
        self.assertIn("Unknown option: --unknown", result.stdout)


if __name__ == "__main__":
    unittest.main()
