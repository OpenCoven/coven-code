"""Exercise the shell installer without network or user configuration writes."""
import hashlib
import io
import os
from pathlib import Path
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

    def run_install(self, *args):
        return subprocess.run(
            ["/bin/bash", str(INSTALLER), "--install-dir", str(self.target), *args],
            env=self.env, text=True, capture_output=True, timeout=10,
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


if __name__ == "__main__":
    unittest.main()
