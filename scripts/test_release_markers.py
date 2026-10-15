#!/usr/bin/env python3
"""Run only the two marker-parser blocks; never execute release side effects."""

import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ReleaseMarkerTests(unittest.TestCase):
    def parse(self, kind, message):
        workflow = (ROOT / ".github/workflows/auto-release.yml").read_text()
        step = workflow.split(f"      - name: Look for --{kind} marker\n", 1)[1]
        body = step.split("        run: |\n", 1)[1]
        lines = []
        for line in body.splitlines():
            if line.strip() and not line.startswith("          "):
                break
            lines.append(line)
        command = textwrap.dedent("\n".join(lines))
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "outputs"
            result = subprocess.run(
                ["bash", "-e", "-c", command], cwd=directory,
                env={"PATH": os.environ["PATH"], "HOME": directory,
                     "MSG": message, "GITHUB_OUTPUT": str(output)},
                capture_output=True, text=True, timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            return dict(line.split("=", 1) for line in output.read_text().splitlines())

    def test_valid_stable_marker_and_first_match(self):
        for message in ["--release v1.2.3", "ship --release v1.2.3 now",
                        "subject\n\n--release\tv1.2.3\nbody",
                        "--release v1.2.3 --release v2.0.0"]:
            with self.subTest(message=message):
                self.assertEqual(self.parse("release", message), {"found": "true", "tag": "v1.2.3"})

    def test_version_suffix_cannot_be_truncated(self):
        for suffix in ["-rc.1", "+build.4", ".4", "oops", "/path", ","]:
            with self.subTest(suffix=suffix):
                self.assertEqual(self.parse("release", "--release v1.2.3" + suffix), {"found": "false"})

    def test_marker_must_be_a_standalone_token(self):
        for message in ["foo--release v1.2.3", "https://host/--release v1.2.3",
                        "--releases v1.2.3", "ordinary commit", "--release", "--release nope"]:
            with self.subTest(message=message):
                self.assertEqual(self.parse("release", message), {"found": "false"})

    def test_release_token_suppresses_patch_even_if_version_unsupported(self):
        for marker in ["--release v1.2.3", "--release v1.2.3-rc.1", "--release nope", "--release"]:
            with self.subTest(marker=marker):
                self.assertEqual(self.parse("patch", "--patch\n" + marker), {"found": "false"})

    def test_patch_standalone_controls(self):
        for message, found in [("fix --patch", "true"), ("--patch\nbody", "true"),
                               ("--patch foo--release v1.2.3", "true"),
                               ("--patches", "false"), ("foo--patch", "false"),
                               ("ordinary commit", "false")]:
            with self.subTest(message=message):
                self.assertEqual(self.parse("patch", message), {"found": found})


if __name__ == "__main__":
    unittest.main()
