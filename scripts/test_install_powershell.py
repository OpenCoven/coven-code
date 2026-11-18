"""Test local PowerShell installs without network or persistent PATH writes."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[1] / "install.ps1"
ROLLBACK_FIXTURE = Path(__file__).resolve().with_name("install-ps1-rollback.test.ps1")
CONSENT_FIXTURE = Path(__file__).resolve().with_name("install-ps1-consent.test.ps1")
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

    def install(self, source, *args):
        result = subprocess.run(
            ["pwsh", "-NoLogo", "-NoProfile", "-File", str(INSTALLER),
             "-Binary", str(source), "-InstallDir", str(self.target), "-NoModifyPath", *args],
            env=self.env, capture_output=True, text=True, timeout=30,
            stdin=subprocess.DEVNULL,
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

    # #369: an existing engine that differs from the one being installed is
    # replaced only with consent.
    def existing(self):
        for name, contents in (("coven-code.exe", b"other engine"),
                               ("coven-cave.exe", b"other alias")):
            (self.target / name).write_bytes(contents)

    def test_a_different_engine_is_kept_without_consent(self):
        self.existing()
        result = self.install(self.source)
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn("Left the existing coven-code in place", output)
        self.assertIn("-Force", output)
        self.assertIn("COVEN_CODE_INSTALL_FORCE=1", output)
        self.assertEqual((self.target / "coven-code.exe").read_bytes(), b"other engine")
        self.assertEqual((self.target / "coven-cave.exe").read_bytes(), b"other alias")
        self.assertEqual(sorted(path.name for path in self.target.iterdir()),
                         ["coven-cave.exe", "coven-code.exe"])

    def test_force_replaces_a_different_engine(self):
        self.existing()
        self.assert_installed(self.install(self.source, "-Force"))

    def test_force_variable_replaces_a_different_engine(self):
        self.existing()
        self.env["COVEN_CODE_INSTALL_FORCE"] = "1"
        self.assert_installed(self.install(self.source))

    def shells(self):
        shells = ["pwsh"]
        if shutil.which("powershell"):
            shells.append("powershell")
        return shells

    def test_a_downloaded_engine_replaces_only_the_same_version_without_consent(self):
        env = {**self.env, "CONSENT_INSTALLER": str(INSTALLER), "CONSENT_TARGET": str(self.target)}
        cases = (("9.9.9", "installed", {}), ("1.0.0", "kept", {}),
                 ("1.0.0", "installed", {"COVEN_CODE_INSTALL_FORCE": "1"}))
        for shell in self.shells():
            for version, expect, extra in cases:
                with self.subTest(shell=shell, version=version, extra=extra):
                    result = subprocess.run(
                        [shell, "-NoProfile", "-NonInteractive", "-File", str(CONSENT_FIXTURE)],
                        env={**env, **extra, "CONSENT_REPORTED_VERSION": version,
                             "CONSENT_EXPECT": expect},
                        capture_output=True, text=True, timeout=30, stdin=subprocess.DEVNULL,
                    )
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertEqual(list(self.staging.iterdir()), [], "download staging leaked")

    def test_alias_failure_restores_existing_install(self):
        for name, contents in (("coven-code.exe", "old engine"),
                               ("coven-cave.exe", "old alias"),
                               ("coven.exe", "old short")):
            (self.target / name).write_text(contents)
        env = {
            **self.env,
            # The rollback under test needs consent to replace the old engine.
            "COVEN_CODE_INSTALL_FORCE": "1",
            "ROLLBACK_INSTALLER": str(INSTALLER),
            "ROLLBACK_TARGET": str(self.target),
            "ROLLBACK_SOURCE": str(self.source),
        }
        for shell in self.shells():
            for mode in ("local", "download"):
                with self.subTest(shell=shell, mode=mode):
                    result = subprocess.run(
                        [shell, "-NoProfile", "-NonInteractive", "-File", str(ROLLBACK_FIXTURE)],
                        env={**env, "ROLLBACK_MODE": mode},
                        capture_output=True, text=True, timeout=30, stdin=subprocess.DEVNULL,
                    )
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertEqual(list(self.staging.iterdir()), [],
                                     "download or local staging leaked")


if __name__ == "__main__":
    unittest.main()
