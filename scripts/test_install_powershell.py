"""Test PowerShell installs without network or persistent PATH writes."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[1] / "install.ps1"
ROLLBACK_FIXTURE = Path(__file__).resolve().with_name("install-ps1-rollback.test.ps1")
CONSENT_FIXTURE = Path(__file__).resolve().with_name("install-ps1-consent.test.ps1")
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
        # A PSModulePath inherited from PowerShell 7 (CI starts this file from
        # it) keeps Windows PowerShell from loading its own modules, such as
        # the one with Get-FileHash. Without the variable each shell builds
        # its default.
        self.env = {name: value for name, value in self.env.items()
                    if name.upper() != "PSMODULEPATH"}
        self.shells = ["pwsh"]
        if shutil.which("powershell"):
            self.shells.append("powershell")

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

    def test_a_downloaded_engine_replaces_only_the_same_version_without_consent(self):
        env = {**self.env, "CONSENT_INSTALLER": str(INSTALLER), "CONSENT_TARGET": str(self.target)}
        cases = (("9.9.9", "installed", {}), ("1.0.0", "kept", {}),
                 ("1.0.0", "installed", {"COVEN_CODE_INSTALL_FORCE": "1"}))
        for shell in self.shells:
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

    def test_engine_version_needs_an_engine_that_names_itself(self):
        not_an_exe = self.target / "coven-code.exe"
        not_an_exe.write_bytes(b"not an executable")
        script = (
            "$env:COVEN_CODE_INSTALLER_TEST = '1'; . $env:CONSENT_INSTALLER; "
            "function coven-code { 'coven-code 1.2.3' }; "
            "function other-tool { 'other-tool 1.2.3' }; "
            "function shouting { 'COVEN-CODE 1.2.3' }; "
            "@((Get-EngineVersion 'coven-code'), (Get-EngineVersion 'other-tool'), "
            "(Get-EngineVersion 'shouting'), (Get-EngineVersion $env:NOT_AN_EXE)) -join ','"
        )
        env = {**self.env, "CONSENT_INSTALLER": str(INSTALLER), "NOT_AN_EXE": str(not_an_exe)}
        for shell in self.shells:
            with self.subTest(shell=shell):
                result = subprocess.run(
                    [shell, "-NoProfile", "-NonInteractive", "-Command", script],
                    env=env, capture_output=True, text=True, timeout=30,
                    stdin=subprocess.DEVNULL,
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(result.stdout.strip().splitlines()[-1],
                                 "1.2.3,unknown,unknown,unknown", result.stdout + result.stderr)

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
        for shell in self.shells:
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
