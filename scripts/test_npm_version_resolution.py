"""Exercise workflow version resolution offline in owned temporary directories."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / '.github/workflows/npm-publish.yml').read_text()


def script(name):
    marker = '      - name: ' + name + '\n'
    if marker not in WORKFLOW:
        return None
    block = WORKFLOW.split(marker, 1)[1].split('\n      - ', 1)[0]
    return '\n'.join(line[10:] for line in block.split('        run: |\n', 1)[1].splitlines())


class Resolution(unittest.TestCase):
    def test_version_is_resolved_once_before_checkout(self):
        self.assertIsNotNone(script('Resolve requested version'))
        self.assertIsNone(script('Resolve version'))
        self.assertEqual(WORKFLOW.count('id: requested_version'), 1)

    def test_resolution(self):
        valid = ['1.2.3', 'v1.2.3', '0.0.0', 'v10.20.300']
        invalid = ['', 'v1.2', '1.2.3-rc.1', '1.2.3+build', 'vv1.2.3', '1.2.3\nextra', '$(touch marker)']
        for value in valid + invalid:
            with self.subTest(value=value), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                output = root / 'output'
                env = {'PATH': '/usr/bin:/bin', 'HOME': directory, 'GITHUB_OUTPUT': str(output), 'REQUESTED_VERSION': value, 'EVENT_NAME': 'workflow_dispatch'}
                def run(body):
                    return subprocess.run(['/bin/bash', '-e', '-o', 'pipefail', '-c', body], cwd=root, env=env, capture_output=True, text=True, timeout=5)
                first = run(script('Resolve requested version'))
                if value in invalid:
                    self.assertNotEqual(first.returncode, 0)
                    self.assertFalse(output.exists())
                    self.assertFalse((root / 'marker').exists())
                    continue
                self.assertEqual(first.returncode, 0, first.stderr)
                fields = dict(line.split('=', 1) for line in output.read_text().splitlines())
                version = value.removeprefix('v')
                self.assertEqual(fields['tag'], 'v' + version)
                self.assertEqual(fields['tag_ref'], 'refs/tags/v' + version)
                second = script('Resolve version')
                if second is not None:
                    output.unlink()
                    env['REQUESTED_TAG'] = fields['tag']
                    result = run(second)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    fields = dict(line.split('=', 1) for line in output.read_text().splitlines())
                self.assertEqual(fields['version'], version)
                self.assertEqual(fields['tag'], 'v' + version)


if __name__ == '__main__':
    unittest.main()
