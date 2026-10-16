"""Run the actual patch-bullet shell step locally without publication commands."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/auto-release.yml"
SHA = "1234567890abcdef1234567890abcdef12345678ab"
REPO = "fixture/owned-repository"


def compose_script():
    workflow = WORKFLOW.read_text()
    marker = "      - name: Compose patch bullet\n"
    assert workflow.count(marker) == 1
    step = workflow.split(marker, 1)[1].split("        run: |\n", 1)[1]
    lines = []
    for line in step.splitlines():
        if line and not line.startswith("          "):
            break
        lines.append(line[10:])
    script = "\n".join(lines)
    assert script.startswith("set -euo pipefail\n")
    return script


class PatchBullet(unittest.TestCase):
    def check_message(self, message, description):
        with tempfile.TemporaryDirectory(prefix="coven-patch-bullet-") as directory:
            root = Path(directory)
            cargo = root / "src-rust/Cargo.toml"
            cargo.parent.mkdir()
            original = b'[workspace.package]\nversion = "1.2.3"\n'
            cargo.write_bytes(original)
            output = root / "output"
            result = subprocess.run(
                ["/bin/bash", "--noprofile", "--norc", "-c", compose_script()],
                cwd=root,
                env={
                    "PATH": "/usr/bin:/bin", "HOME": directory,
                    "MSG": message, "SHA": SHA, "REPO": REPO,
                    "GITHUB_OUTPUT": str(output),
                },
                capture_output=True, text=True, timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            bullet = f"- {description} ([`{SHA[:7]}`](https://github.com/{REPO}/commit/{SHA}))"
            self.assertEqual(output.read_text(), f"tag=v1.2.3\nbullet<<__EOF__\n{bullet}\n__EOF__\n")
            self.assertEqual(result.stdout, f"Will patch v1.2.3 with:\n  {bullet}\n")
            self.assertEqual(result.stderr, "")
            self.assertEqual(cargo.read_bytes(), original)
            self.assertEqual(sorted(path.name for path in root.iterdir()), ["output", "src-rust"])

    def test_large_bodies_do_not_break_subject_extraction(self):
        # Each environment value stays below Linux's per-string exec limit.
        for length in (100_000, 120_000):
            with self.subTest(body_bytes=length):
                self.check_message("fix output --patch\n\n" + ("body\n" * (length // 5)), "fix output")

    def test_first_line_and_marker_behavior(self):
        cases = [
            ("fix output --patch", "fix output"),
            ("fix output --patch\n", "fix output"),
            ("fix output --patch\nbody ignored\n", "fix output"),
            ("  --patch fix output --patch  \nbody", "fix output"),
            ("subject without marker\n--patch", "subject without marker"),
            ("--patch\nbody", "(patch — see commit)"),
            ("\nbody --patch", "(patch — see commit)"),
            ("", "(patch — see commit)"),
        ]
        for message, expected in cases:
            with self.subTest(message=message):
                self.check_message(message, expected)

    def test_subject_is_literal_shell_data(self):
        subject = "fix $(touch injected) `touch injected2` ; & < > 'quoted' --patch"
        self.check_message(subject + "\nbody", subject.removesuffix(" --patch"))


if __name__ == "__main__":
    unittest.main()
