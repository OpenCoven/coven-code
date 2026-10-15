"""Exercise shell-profile directory creation with isolated local installs."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"


class InstallerConfigDirectoryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="coven-install-config-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.home = self.root / "home"
        self.home.mkdir()
        self.commands = self.root / "commands"
        self.commands.mkdir()
        for command in ("uname", "tr", "mkdir", "cp", "chmod", "mv", "ln", "grep", "dirname"):
            executable = shutil.which(command)
            self.assertIsNotNone(executable, command)
            (self.commands / command).symlink_to(executable)
        curl = self.commands / "curl"
        curl.write_text('#!/bin/sh\nprintf called > "$NETWORK_MARKER"\nexit 99\n')
        curl.chmod(0o755)
        self.binary = self.root / "local-binary"
        self.binary.write_text("#!/bin/sh\nprintf 'fixture binary\\n'\n")
        self.target = self.root / "installed"
        self.network_marker = self.root / "network-called"
        self.profile = self.home / ".config/fish/config.fish"

    def install(self):
        result = subprocess.run(
            ["/bin/bash", str(INSTALLER), "--version", "0.1.4", "--binary",
             str(self.binary), "--install-dir", str(self.target)],
            env={"HOME": str(self.home), "SHELL": "/fixture/bin/fish",
                 "PATH": str(self.commands), "NETWORK_MARKER": str(self.network_marker)},
            text=True, capture_output=True, timeout=10,
        )
        self.assertFalse(self.network_marker.exists(), "local pinned install must not use network")
        return result

    def test_missing_nested_profile_parent(self):
        self.assertFalse(self.profile.parent.exists())
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(str(self.target), self.profile.read_text())
        self.assertEqual((self.target / "coven-code").read_bytes(), self.binary.read_bytes())
        self.assertEqual((self.target / "coven-cave").read_bytes(), self.binary.read_bytes())

    def test_existing_profile_preserved_and_repeat_is_idempotent(self):
        self.profile.parent.mkdir(parents=True)
        original = "# Existing user configuration\nset -gx EDITOR vi\n"
        self.profile.write_text(original)
        first = self.install()
        self.assertEqual(first.returncode, 0, first.stderr)
        contents = self.profile.read_text()
        self.assertTrue(contents.startswith(original))
        self.assertEqual(contents.count(str(self.target)), 1)
        second = self.install()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(self.profile.read_text(), contents)

    def test_blocked_parent_reports_failure_without_clobbering(self):
        blocked = self.home / ".config"
        blocked.write_text("preserve me")
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(str(blocked), result.stderr)
        self.assertEqual(blocked.read_text(), "preserve me")
        self.assertFalse(self.profile.exists())
        # Installation precedes profile setup; this unit does not add rollback.
        self.assertEqual((self.target / "coven-code").read_bytes(), self.binary.read_bytes())


if __name__ == "__main__":
    unittest.main()
