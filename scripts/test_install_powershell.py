"""Test local PowerShell installs without network or persistent PATH writes."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[1] / "install.ps1"
ROLLBACK_FIXTURE = Path(__file__).resolve().with_name("install-ps1-rollback.test.ps1")
ENGINE = b"fixture engine; never executed"


@unittest.skipUnless(shutil.which("pwsh"), "PowerShell is required")
class PowerShellInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="coven-powershell-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.target = self.root / "installed engine"
        self.target.mkdir()
        self.source = self.root / "source.exe"
        self.source.write_bytes(ENGINE)
        self.staging = self.root / "staging"
        self.staging.mkdir()
        self.env = {**os.environ, "USERPROFILE": str(self.root), "GITHUB_ACTIONS": "false"}
        self.env.update({name: str(self.staging) for name in ("TMPDIR", "TMP", "TEMP")})
        self.env.pop("GITHUB_PATH", None)

    def install(self, source):
        result = subprocess.run(
            ["pwsh", "-NoLogo", "-NoProfile", "-File", str(INSTALLER),
             "-Binary", str(source), "-InstallDir", str(self.target), "-NoModifyPath"],
            env=self.env, capture_output=True, text=True, timeout=30,
        )
        self.assertEqual(list(self.staging.iterdir()), [], "staged files must be cleaned up")
        return result

    def assert_installed(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in ("coven-code.exe", "coven-cave.exe"):
            self.assertEqual((self.target / name).read_bytes(), ENGINE)

    def test_fresh_install(self):
        self.assert_installed(self.install(self.source))

    def test_reinstall_from_installed_engine(self):
        self.assert_installed(self.install(self.source))
        self.assert_installed(self.install(self.target / "coven-code.exe"))

    def test_reinstall_from_installed_alias(self):
        self.assert_installed(self.install(self.source))
        self.assert_installed(self.install(self.target / "coven-cave.exe"))

    def test_missing_source_preserves_existing_install(self):
        self.assert_installed(self.install(self.source))
        result = self.install(self.root / "missing.exe")
        self.assertNotEqual(result.returncode, 0)
        for name in ("coven-code.exe", "coven-cave.exe"):
            self.assertEqual((self.target / name).read_bytes(), ENGINE)

    def test_source_copy_failure_preserves_existing_install(self):
        self.assert_installed(self.install(self.source))
        invalid_source = self.root / "directory.exe"
        invalid_source.mkdir()
        result = self.install(invalid_source)
        self.assertNotEqual(result.returncode, 0)
        for name in ("coven-code.exe", "coven-cave.exe"):
            self.assertEqual((self.target / name).read_bytes(), ENGINE)

    def test_alias_failure_restores_existing_install(self):
        for name, contents in (("coven-code.exe", "old engine"),
                               ("coven-cave.exe", "old alias"),
                               ("coven.exe", "old short")):
            (self.target / name).write_text(contents)
        env = {
            **self.env,
            "ROLLBACK_INSTALLER": str(INSTALLER),
            "ROLLBACK_TARGET": str(self.target),
            "ROLLBACK_SOURCE": str(self.source),
        }
        shells = ["pwsh"]
        if shutil.which("powershell"):
            shells.append("powershell")
        for shell in shells:
            for mode in ("local", "download"):
                with self.subTest(shell=shell, mode=mode):
                    result = subprocess.run(
                        [shell, "-NoProfile", "-NonInteractive", "-File", str(ROLLBACK_FIXTURE)],
                        env={**env, "ROLLBACK_MODE": mode},
                        capture_output=True, text=True, timeout=30,
                    )
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertEqual(list(self.staging.iterdir()), [],
                                     "download or local staging leaked")


if __name__ == "__main__":
    unittest.main()
