"""Exercise the shell installer without network or user configuration writes."""
import hashlib
import io
import os
from pathlib import Path
import pty
import subprocess
import tarfile
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"
BINARY = b"#!/bin/sh\nprintf 'fixture 1.2.3\\n'\n"


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="coven-install-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / "home"
        self.home.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.target = self.root / "installed engine"
        self.local = self.root / "local engine"
        self.local.write_bytes(BINARY)
        self.archive = self.root / "fixture.tar.gz"
        with tarfile.open(self.archive, "w:gz") as archive:
            entry = tarfile.TarInfo("coven-code")
            entry.size = len(BINARY)
            archive.addfile(entry, io.BytesIO(BINARY))
        # The installer verifies the archive against the release SHA256SUMS,
        # which must list all five release archives.
        digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        self.sums = self.root / "SHA256SUMS"
        self.sums.write_text("".join(f"{digest}  coven-code-{name}\n" for name in (
            "linux-aarch64.tar.gz", "linux-x86_64.tar.gz", "macos-aarch64.tar.gz",
            "macos-x86_64.tar.gz", "windows-x86_64.zip",
        )))
        self.log = self.root / "curl.log"
        self.env = {
            **os.environ,
            "HOME": str(self.home), "SHELL": "/bin/bash",
            "PATH": f"{self.bin}:/usr/bin:/bin",
            "FIXTURE_ARCHIVE": str(self.archive), "FIXTURE_SUMS": str(self.sums),
            "CURL_LOG": str(self.log),
        }
        # No real curl can run: every invocation is logged and either rejected
        # (offline mode) or answered from this temporary fixture.
        curl = self.bin / "curl"
        curl.write_text('''#!/bin/bash
printf '%s\\n' "$*" >> "$CURL_LOG"
if [[ "${OFFLINE:-0}" == 1 ]]; then exit 77; fi
if [[ "$*" == *api.github.com* ]]; then
  printf '{"tag_name": "v1.2.3"}\\n'
else
  fixture=$FIXTURE_ARCHIVE
  if [[ "$*" == */SHA256SUMS* ]]; then fixture=$FIXTURE_SUMS; fi
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == -o ]]; then cp "$fixture" "$2"; exit; fi
    shift
  done
  exit 78
fi
''')
        curl.chmod(0o755)

    def run_install(self, *args, stdin=subprocess.DEVNULL):
        return subprocess.run(
            ["/bin/bash", str(INSTALLER), "--install-dir", str(self.target), *args],
            env=self.env, text=True, capture_output=True, timeout=10, stdin=stdin,
        )

    def assert_installed(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.target / "coven-code").read_bytes(), BINARY)
        self.assertEqual((self.target / "coven-cave").read_bytes(), BINARY)
        self.assertTrue(os.access(self.target / "coven-code", os.X_OK))
        self.assertIn(str(self.target), (self.home / ".bashrc").read_text())

    def test_local_binary_without_version_is_offline(self):
        self.env["OFFLINE"] = "1"
        self.assert_installed(self.run_install("--binary", str(self.local)))
        self.assertFalse(self.log.exists())

    def test_local_binary_with_version_is_offline(self):
        self.env["OFFLINE"] = "1"
        self.assert_installed(self.run_install("--binary", str(self.local), "--version", "1.2.3"))
        self.assertFalse(self.log.exists())

    def test_pinned_download(self):
        self.assert_installed(self.run_install("--version", "1.2.3"))
        requests = self.log.read_text().splitlines()
        self.assertEqual(len(requests), 2)
        self.assertIn("/releases/download/v1.2.3/SHA256SUMS", requests[0])
        self.assertIn("/releases/download/v1.2.3/", requests[1])

    def test_latest_download(self):
        self.assert_installed(self.run_install())
        requests = self.log.read_text().splitlines()
        self.assertEqual(len(requests), 3)
        self.assertIn("/releases/latest", requests[0])
        self.assertIn("/releases/download/v1.2.3/SHA256SUMS", requests[1])
        self.assertIn("/releases/download/v1.2.3/", requests[2])

    def assert_alias_failure_preserves_install(self, *args):
        self.target.mkdir()
        old_binary = b"old executable\n"
        (self.target / "coven-code").write_bytes(old_binary)
        (self.target / "coven-cave").symlink_to("coven-code")
        self.env["BLOCK_ALIAS"] = str(self.target / "coven-cave")
        (self.bin / "ln").write_text("#!/bin/sh\nexit 73\n")
        (self.bin / "cp").write_text('''#!/bin/bash
if [[ "${!#}" == "$BLOCK_ALIAS" ]]; then exit 73; fi
exec /bin/cp "$@"
''')
        (self.bin / "mv").write_text('''#!/bin/bash
if [[ "$1" == */.coven-code-stage-*/coven-cave && "${!#}" == "$BLOCK_ALIAS" ]]; then exit 73; fi
exec /bin/mv "$@"
''')
        for name in ("ln", "cp", "mv"):
            (self.bin / name).chmod(0o755)

        result = self.run_install(*args)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.target / "coven-code").read_bytes(), old_binary)
        self.assertTrue((self.target / "coven-cave").is_symlink())
        self.assertEqual(os.readlink(self.target / "coven-cave"), "coven-code")
        self.assertEqual(sorted(path.name for path in self.target.iterdir()),
                         ["coven-cave", "coven-code"])

    def test_local_alias_failure_preserves_install(self):
        self.assert_alias_failure_preserves_install("--binary", str(self.local), "--force")

    def test_download_alias_failure_preserves_install(self):
        self.assert_alias_failure_preserves_install("--version", "1.2.3", "--force")

    # #369: an existing engine that differs from the one being installed is
    # replaced only with consent.
    OTHER = b"#!/bin/sh\nprintf 'fixture 1.0.0\\n'\n"
    SAME_VERSION = b"#!/bin/sh\n# another build\nprintf 'fixture 1.2.3\\n'\n"

    def existing(self, contents):
        self.target.mkdir()
        (self.target / "coven-code").write_bytes(contents)
        (self.target / "coven-code").chmod(0o755)
        (self.target / "coven-cave").symlink_to("coven-code")

    def assert_kept(self, result, contents):
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("Left the existing coven-code in place", result.stderr)
        self.assertIn("--force", result.stderr)
        self.assertIn("COVEN_CODE_INSTALL_FORCE=1", result.stderr)
        self.assertNotIn("installed to", result.stdout)
        self.assertEqual((self.target / "coven-code").read_bytes(), contents)
        self.assertEqual(os.readlink(self.target / "coven-cave"), "coven-code")
        self.assertEqual(sorted(path.name for path in self.target.iterdir()),
                         ["coven-cave", "coven-code"])
        self.assertFalse((self.home / ".bashrc").exists(), "a refused install edited the shell profile")

    def test_a_different_local_engine_is_kept_without_consent(self):
        self.existing(self.OTHER)
        self.assert_kept(self.run_install("--binary", str(self.local)), self.OTHER)

    def test_a_different_downloaded_engine_is_kept_without_consent(self):
        self.existing(self.OTHER)
        result = self.run_install("--version", "1.2.3")
        self.assert_kept(result, self.OTHER)
        self.assertIn("v1.0.0", result.stderr)

    def test_force_replaces_a_different_engine(self):
        self.existing(self.OTHER)
        self.assert_installed(self.run_install("--binary", str(self.local), "--force"))

    def test_force_variable_replaces_a_different_engine(self):
        self.existing(self.OTHER)
        self.env["COVEN_CODE_INSTALL_FORCE"] = "1"
        self.assert_installed(self.run_install("--version", "1.2.3"))

    def test_the_same_engine_is_reinstalled_without_asking(self):
        self.existing(BINARY)
        self.assert_installed(self.run_install("--binary", str(self.local)))
        self.assert_installed(self.run_install("--version", "1.2.3"))

    def test_the_same_version_is_reinstalled_without_asking(self):
        self.existing(self.SAME_VERSION)
        self.assert_installed(self.run_install("--version", "1.2.3"))

    def run_on_terminal(self, answer, *args):
        primary, secondary = pty.openpty()
        try:
            # The terminal holds the answer until the installer reads it.
            os.write(primary, answer)
            return self.run_install(*args, stdin=secondary)
        finally:
            os.close(secondary)
            os.close(primary)

    def test_a_terminal_answer_decides(self):
        self.existing(self.OTHER)
        result = self.run_on_terminal(b"n\n", "--binary", str(self.local))
        self.assertIn("Replace it? [y/N]", result.stderr)
        self.assert_kept(result, self.OTHER)
        result = self.run_on_terminal(b"y\n", "--binary", str(self.local))
        self.assertIn("Replace it? [y/N]", result.stderr)
        self.assert_installed(result)

    # A release binary that the system cannot run: the loader prints why and
    # the process exits 127. These are the two messages a Linux system without
    # ALSA, or with an older glibc, prints for the published archives.
    NO_ALSA = ("coven-code: error while loading shared libraries: libasound.so.2: "
               "cannot open shared object file: No such file or directory")
    OLD_GLIBC = ("coven-code: /lib/aarch64-linux-gnu/libc.so.6: version `GLIBC_2.39' "
                 "not found (required by coven-code)")

    def as_linux(self):
        # The start check is Linux-only; these tests also run on macOS.
        uname = self.bin / "uname"
        uname.write_text("#!/bin/sh\n[ \"$1\" = -s ] && echo Linux || echo aarch64\n")
        uname.chmod(0o755)

    def unstartable(self, message):
        self.as_linux()
        # A quoted here-document keeps the loader's backtick literal.
        return f"#!/bin/sh\ncat >&2 <<'LOADER'\n{message}\nLOADER\nexit 127\n".encode()

    def assert_not_installed(self, result, *expected):
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("cannot start on this system, so nothing was installed", result.stderr)
        for text in expected:
            self.assertIn(text, result.stderr)
        self.assertIn("docs/installation.md#linux-runtime-requirements", result.stderr)
        self.assertNotIn("installed to", result.stdout)
        self.assertFalse((self.home / ".bashrc").exists(), "a failed install edited the shell profile")

    def test_local_binary_that_cannot_start_is_not_installed(self):
        self.local.write_bytes(self.unstartable(self.NO_ALSA))
        result = self.run_install("--binary", str(self.local))
        self.assert_not_installed(result, "libasound.so.2: cannot open shared object file",
                                  "The ALSA runtime library (libasound.so.2) is missing")
        self.assertNotIn("newer glibc", result.stderr)
        self.assertEqual(list(self.target.iterdir()), [])

    def test_every_missing_requirement_is_named_in_one_run(self):
        # The loader reports only the missing library; ldd also lists the
        # glibc versions the binary needs.
        self.local.write_bytes(self.unstartable(self.NO_ALSA))
        ldd = self.bin / "ldd"
        ldd.write_text("#!/bin/sh\ncat <<'LDD'\n"
                       "coven-code: /lib/libc.so.6: version `GLIBC_2.39' not found (required by coven-code)\n"
                       "\tlibasound.so.2 => not found\n"
                       "\tlibc.so.6 => /lib/libc.so.6 (0x0000ffff00000000)\n"
                       "LDD\n")
        ldd.chmod(0o755)
        result = self.run_install("--binary", str(self.local))
        self.assert_not_installed(result, "version `GLIBC_2.39' not found",
                                  "The binary needs a newer glibc than this system has",
                                  "The ALSA runtime library (libasound.so.2) is missing")
        self.assertNotIn("libc.so.6 =>", result.stderr)
        self.assertEqual(list(self.target.iterdir()), [])

    def test_downloaded_binary_that_cannot_start_keeps_the_prior_install(self):
        binary = self.unstartable(self.OLD_GLIBC)
        with tarfile.open(self.archive, "w:gz") as archive:
            entry = tarfile.TarInfo("coven-code")
            entry.size = len(binary)
            archive.addfile(entry, io.BytesIO(binary))
        digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        self.sums.write_text("".join(f"{digest}  coven-code-{name}\n" for name in (
            "linux-aarch64.tar.gz", "linux-x86_64.tar.gz", "macos-aarch64.tar.gz",
            "macos-x86_64.tar.gz", "windows-x86_64.zip",
        )))
        self.target.mkdir()
        (self.target / "coven-code").write_bytes(BINARY)
        (self.target / "coven-cave").symlink_to("coven-code")

        result = self.run_install("--version", "1.2.3")
        self.assert_not_installed(result, "version `GLIBC_2.39' not found",
                                  "The binary needs a newer glibc than this system has")
        self.assertNotIn("ALSA", result.stderr)
        self.assertEqual((self.target / "coven-code").read_bytes(), BINARY)
        self.assertEqual(os.readlink(self.target / "coven-cave"), "coven-code")
        self.assertEqual(sorted(path.name for path in self.target.iterdir()),
                         ["coven-cave", "coven-code"])

    def test_a_binary_that_starts_is_installed_on_linux(self):
        self.as_linux()
        self.assert_installed(self.run_install("--binary", str(self.local)))
        self.assert_installed(self.run_install("--version", "1.2.3"))


if __name__ == "__main__":
    unittest.main()
