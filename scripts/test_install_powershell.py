"""Test PowerShell installs without network or persistent PATH writes."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[1] / "install.ps1"
ROLLBACK_FIXTURE = Path(__file__).resolve().with_name("install-ps1-rollback.test.ps1")
DOWNLOAD_FIXTURE = Path(__file__).resolve().with_name("install-ps1-download.test.ps1")
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
        self.shells = ["pwsh"]
        if shutil.which("powershell"):
            self.shells.append("powershell")

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
        for shell in self.shells:
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

    def download(self, shell, body):
        """Run the installer's HTTP download and install against in-process responses."""
        target = self.root / f"download {shell} {body}"
        target.mkdir()
        result = subprocess.run(
            [shell, "-NoProfile", "-NonInteractive", "-File", str(DOWNLOAD_FIXTURE)],
            env={**self.env, "DOWNLOAD_INSTALLER": str(INSTALLER), "DOWNLOAD_TARGET": str(target),
                 "DOWNLOAD_SOURCE": str(self.source), "DOWNLOAD_BODY": body},
            capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(list(self.staging.iterdir()), [], "download staging leaked")
        return target, result

    def test_download_prints_only_installer_messages(self):
        expected = ("Installing coven-code v9.9.9 ", "Downloading https://", "Verified SHA-256 ",
                    "Extracting...", "Installed: ", "Installed alias: ")
        for shell in self.shells:
            with self.subTest(shell=shell):
                target, result = self.download(shell, "ok")
                # Shown so a hosted run's log carries the output it accepted.
                print(f"\n[{shell}] download install output:\n{result.stdout}", flush=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                lines = [line for line in result.stdout.splitlines() if line.strip()]
                self.assertEqual([line for line in lines if not line.startswith(expected)], [],
                                 "the download wrote a line that is not an installer message")
                self.assertEqual(len(lines), len(expected), result.stdout)
                for line, prefix in zip(lines, expected):
                    self.assertTrue(line.startswith(prefix), result.stdout)
                for name in ("coven-code.exe", "coven-cave.exe"):
                    self.assertEqual((target / name).read_bytes(), ENGINE)

    def test_failed_download_body_fails_the_install(self):
        for shell in self.shells:
            with self.subTest(shell=shell):
                target, result = self.download(shell, "fail")
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                # The copy's own error, not a later manifest or checksum
                # failure on a file that was cut short.
                self.assertRegex(result.stdout, "Download failed: .*fixture body failure")
                self.assertEqual(list(target.iterdir()), [], "a failed download installed files")


if __name__ == "__main__":
    unittest.main()
