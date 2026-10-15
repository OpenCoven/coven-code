"""Check release rejection paths and the real version stamper in a disposable copy."""
import contextlib
import io
import json
import re
from pathlib import Path
import runpy
import shutil
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
CHECK = runpy.run_path(str(ROOT / 'scripts/check-release-version.py'))


class ReleaseVersion(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='release-version-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        paths = ['scripts/bump-version.py', 'src-rust/Cargo.toml', 'src-rust/Cargo.lock',
                 'npm/package.json', 'README.md', 'docs/index.md', 'docs/installation.md',
                 'src-rust/crates/acp/registry-template/agent.json']
        paths += [str(p.relative_to(ROOT)) for p in (ROOT / 'src-rust/crates').glob('*/Cargo.toml')]
        for name in paths:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
        stamp = runpy.run_path(str(self.root / 'scripts/bump-version.py'))
        with mock.patch('sys.argv', ['bump-version.py', 'v0.9.0']), contextlib.redirect_stdout(io.StringIO()):
            stamp['main']()

    def test_real_stamp_passes(self):
        CHECK['check_release'](self.root, 'v0.9.0', ['v0.8.0', 'unrelated'])

    def test_readme_engine_stamp_preserves_upstream_attribution(self):
        original = (ROOT / 'README.md').read_text()
        stamped = (self.root / 'README.md').read_text()
        self.assertIn('`coven-code` **v0.9.0**', stamped)
        self.assertEqual(re.search(r'Claurst v[0-9.]+', original).group(),
                         re.search(r'Claurst v[0-9.]+', stamped).group())

    def test_stale_readme_engine_stamp_rejects(self):
        p = self.root / 'README.md'
        p.write_text(p.read_text().replace('`coven-code` **v0.9.0**', '`coven-code` **v0.8.0**'))
        with self.assertRaisesRegex(ValueError, 'README.md'):
            CHECK['check_release'](self.root, 'v0.9.0', [])

    def test_equal_older_and_prerelease_tags_reject(self):
        for tag in ['v0.9.0', 'v0.10.0', 'v0.9.1-rc.1', 'v0.9.0+metadata']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                CHECK['check_release'](self.root, 'v0.9.0', [tag])
        CHECK['check_release'](self.root, 'v0.9.0', ['v0.9.0-rc.2'])

    def test_newer_tag_after_preflight_rejects_publication(self):
        # The same candidate passes early preflight, then must fail when a
        # newer release appears during its build. No stale result is cached.
        CHECK['check_release'](self.root, 'v0.9.0', ['v0.8.0'])
        with self.assertRaisesRegex(ValueError, 'strictly newer than v0.10.0'):
            CHECK['check_release'](self.root, 'v0.9.0', ['v0.8.0', 'v0.10.0'])

    def test_semver_precedence(self):
        tags = ['v1.0.0-alpha', 'v1.0.0-alpha.1', 'v1.0.0-alpha.beta',
                'v1.0.0-beta', 'v1.0.0-beta.2', 'v1.0.0-beta.11', 'v1.0.0-rc.1', 'v1.0.0', 'v1.0.1']
        self.assertEqual(sorted(reversed(tags), key=CHECK['precedence']), tags)
        for tag in ['v01.0.0', 'v1.0.0-01', '1.0.0', 'v1.0', 'v1.0.0-']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                CHECK['precedence'](tag)

    def test_remote_failure_rejects(self):
        import subprocess
        with mock.patch('sys.argv', ['check-release-version.py', 'v0.9.0']), \
                mock.patch('subprocess.run', side_effect=subprocess.CalledProcessError(128, 'git')), \
                contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(CHECK['main'](), 1)

    def test_workspace_lock_drift_rejects(self):
        # Corrupt only a disposable fixture, never the checkout's generated lock.
        p = self.root / 'src-rust/Cargo.lock'
        text = p.read_text().replace('name = "claurst"\nversion = "0.9.0"',
                                     'name = "claurst"\nversion = "0.8.0"')
        p.write_text(text)
        with self.assertRaisesRegex(ValueError, 'Cargo.lock claurst'):
            CHECK['check_release'](self.root, 'v0.9.0', [])

    def test_package_version_drift_rejects(self):
        p = self.root / 'npm/package.json'
        data = json.loads(p.read_text()); data['version'] = '0.8.0'
        p.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'npm'):
            CHECK['check_release'](self.root, 'v0.9.0', ['v0.8.0'])

    def test_registry_url_drift_rejects(self):
        p = self.root / 'src-rust/crates/acp/registry-template/agent.json'
        data = json.loads(p.read_text())
        data['distribution']['binary']['linux-x86_64']['archive'] = 'https://example.invalid/archive'
        p.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'ACP archive'):
            CHECK['check_release'](self.root, 'v0.9.0', [])

    def test_doc_drift_rejects(self):
        (self.root / 'docs/index.md').write_text('**Version:** 0.8.0\n')
        with self.assertRaisesRegex(ValueError, 'docs/index.md'):
            CHECK['check_release'](self.root, 'v0.9.0', [])



class ForwardOnly(unittest.TestCase):
    """Auto-release ordering check, run before any stamp exists."""

    def remote(self, *tags):
        import subprocess
        refs = ''.join(f'0000000000000000000000000000000000000000\trefs/tags/{t}\n' for t in tags)
        return mock.patch('subprocess.run', return_value=subprocess.CompletedProcess(
            ['git'], 0, stdout=refs, stderr=''))

    def run_cli(self, *args):
        with mock.patch('sys.argv', ['check-release-version.py', *args]), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return CHECK['main']()

    def test_first_release_and_unrelated_tags(self):
        CHECK['check_forward']('v0.1.0', [])
        CHECK['check_forward']('v0.1.0', ['nightly', 'latest'])

    def test_malformed_version_shaped_tags_fail_closed(self):
        for tag in ['v1.0', 'v9.0.0-01']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                CHECK['check_forward']('v0.1.0', [tag])

    def test_stable_order_equal_lower_and_numeric_components(self):
        for candidate, accepted in [('v1.9.9', False), ('v1.0.0', False),
                                    ('v1.10.0', True), ('v2.0.0', True)]:
            with self.subTest(candidate=candidate):
                if accepted:
                    CHECK['check_forward'](candidate, ['v1.9.9'])
                else:
                    with self.assertRaises(ValueError):
                        CHECK['check_forward'](candidate, ['v1.9.9'])

    def test_semver_prerelease_sequence(self):
        # SemVer 2.0.0 section 11 example, including numeric vs lexical IDs.
        sequence = ['alpha', 'alpha.1', 'alpha.beta', 'beta', 'beta.2', 'beta.11', 'rc.1']
        tags = [f'v1.0.0-{suffix}' for suffix in sequence] + ['v1.0.0']
        for previous, candidate in zip(tags, tags[1:]):
            with self.subTest(previous=previous, candidate=candidate):
                CHECK['check_forward'](candidate, [previous])
                with self.assertRaises(ValueError):
                    CHECK['check_forward'](previous, [candidate])

    def test_cli_forward_only_skips_unwritten_stamps(self):
        # The checkout is still stamped with the previous version, as it is in
        # auto-release before bump-version.py runs.
        with self.remote('v0.8.0', 'v0.8.0^{}'):
            self.assertEqual(self.run_cli('--forward-only', 'v0.9.0'), 0)
            self.assertEqual(self.run_cli('v0.9.0'), 1)
            self.assertEqual(self.run_cli('--forward-only', 'v0.8.0'), 1)

    def test_cli_forward_only_remote_failure_rejects(self):
        import subprocess
        with mock.patch('subprocess.run', side_effect=subprocess.CalledProcessError(128, 'git')):
            self.assertEqual(self.run_cli('--forward-only', 'v0.9.0'), 1)

if __name__ == '__main__':
    unittest.main()
