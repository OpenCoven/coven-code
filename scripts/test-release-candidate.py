"""Hold the release-candidate workflow to its contract and exercise the smoke checks."""
import hashlib
import io
import os
from pathlib import Path
import re
import runpy
import tarfile
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SMOKE = runpy.run_path(str(Path(__file__).with_name('smoke-release-archive.py')))
PACK = runpy.run_path(str(Path(__file__).with_name('package-release.py')))
CANDIDATE = (ROOT / '.github/workflows/release-candidate.yml').read_text(encoding='utf-8')
RELEASE = (ROOT / '.github/workflows/release.yml').read_text(encoding='utf-8')


def job(workflow, name):
    """The text of one job: from its key to the next job's comment or key."""
    match = re.search(rf'^  {name}:\n(.*?)(?=^  (?:# |[a-z-]+:\n)|\Z)', workflow, re.MULTILINE | re.DOTALL)
    if not match:
        raise AssertionError(f'missing job: {name}')
    return match[1].rstrip('\n')


class WorkflowContract(unittest.TestCase):
    def test_build_job_is_the_release_build_job(self):
        release = job(RELEASE, 'build').replace('needs: preflight', 'needs: contract', 1)
        self.assertEqual(job(CANDIDATE, 'build'), release)

    def test_packaging_commands_are_the_release_commands(self):
        for command in ('python scripts/package-release.py artifacts release',
                        'node scripts/generate-release-checksums.mjs release'):
            self.assertIn(command, job(RELEASE, 'release'))
            self.assertIn(command, job(CANDIDATE, 'package'))

    def test_token_is_read_only_and_nothing_publishes(self):
        self.assertEqual(re.findall(r'^permissions:\n((?:  .*\n)+)', CANDIDATE, re.MULTILINE),
                         ['  contents: read\n'])
        self.assertNotIn('permissions:', CANDIDATE.split('\njobs:\n', 1)[1])
        for forbidden in ('action-gh-release', 'gh release', 'gh workflow run', 'gh api',
                          'secrets.', 'git push', 'git tag', 'npm publish', ': write'):
            self.assertNotIn(forbidden, CANDIDATE)

    def test_every_packaged_artifact_runs_on_its_own_platform(self):
        smoke = job(CANDIDATE, 'smoke')
        pairs = dict(re.findall(r'- artifact: (\S+)\n\s+os: (\S+)', smoke))
        self.assertEqual(set(pairs), set(PACK['ARTIFACTS']))
        self.assertEqual(set(pairs), set(SMOKE['ARTIFACTS']))
        self.assertEqual(len(set(pairs.values())), 5)
        for artifact, runner in pairs.items():
            family = artifact.split('-')[2]
            self.assertTrue(runner.startswith({'linux': 'ubuntu'}.get(family, family)), (artifact, runner))


class ArchiveVerification(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='release-candidate-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.release = self.root / 'release'
        self.release.mkdir()
        self.scratch = self.root / 'scratch'
        self.scratch.mkdir()

    def tarball(self, members, artifact='coven-code-linux-x86_64'):
        path = self.release / f'{artifact}.tar.gz'
        with tarfile.open(path, 'w:gz') as archive:
            for name, payload in members:
                info = tarfile.TarInfo(name)
                if payload is None:
                    info.type, info.linkname = tarfile.SYMTYPE, '/bin/sh'
                    archive.addfile(info)
                else:
                    info.size = len(payload)
                    archive.addfile(info, io.BytesIO(payload))
        return path

    def manifest(self, *paths, digest=None):
        lines = [f'{digest or hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}' for path in paths]
        (self.release / 'SHA256SUMS').write_text('\n'.join(lines) + '\n', encoding='utf-8')

    def verify(self, artifact='coven-code-linux-x86_64'):
        return SMOKE['verify_archive'](self.release, artifact, self.scratch)

    def test_extracts_the_single_verified_member(self):
        self.manifest(self.tarball([('coven-code', b'inert')]))
        binary = self.verify()
        self.assertEqual(binary.read_bytes(), b'inert')
        self.assertEqual(binary.name, 'coven-code')

    def test_windows_archive_holds_only_the_exe(self):
        path = self.release / 'coven-code-windows-x86_64.zip'
        with zipfile.ZipFile(path, 'w') as archive:
            archive.writestr('coven-code.exe', b'inert')
        self.manifest(path)
        self.assertEqual(self.verify('coven-code-windows-x86_64').name, 'coven-code.exe')
        with zipfile.ZipFile(path, 'w') as archive:
            archive.writestr('coven-code.exe', b'inert')
            archive.writestr('extra.dll', b'inert')
        self.manifest(path)
        with self.assertRaisesRegex(ValueError, 'expected only coven-code.exe'):
            self.verify('coven-code-windows-x86_64')

    def test_rejects_a_digest_that_differs_from_the_manifest(self):
        self.manifest(self.tarball([('coven-code', b'inert')]), digest='0' * 64)
        with self.assertRaisesRegex(ValueError, 'does not match the manifest'):
            self.verify()
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_rejects_a_missing_or_repeated_manifest_entry(self):
        path = self.tarball([('coven-code', b'inert')])
        (self.release / 'SHA256SUMS').write_text('', encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'no entry'):
            self.verify()
        self.manifest(path, path)
        with self.assertRaisesRegex(ValueError, 'malformed or repeated'):
            self.verify()

    def test_rejects_extra_renamed_linked_and_empty_members(self):
        for members in ([('coven-code', b'inert'), ('extra', b'inert')], [('bin/coven-code', b'inert')],
                        [('coven-code', None)], [('coven-code', b'')]):
            self.manifest(self.tarball(members))
            with self.assertRaises(ValueError, msg=str(members)):
                self.verify()

    def test_rejects_an_unsupported_artifact_name(self):
        with self.assertRaisesRegex(ValueError, 'not one of the five'):
            self.verify('coven-code-windows-aarch64')

    @unittest.skipIf(os.name == 'nt', 'the inert binary is a shell script')
    def test_version_must_equal_the_source_stamp(self):
        binary = self.scratch / 'coven-code'
        binary.write_text('#!/bin/sh\necho "coven-code 1.2.3"\n', encoding='utf-8')
        binary.chmod(0o755)
        self.assertEqual(SMOKE['check_version'](binary, '1.2.3', self.scratch), 'coven-code 1.2.3')
        with self.assertRaisesRegex(ValueError, 'expected "coven-code 1.2.4"'):
            SMOKE['check_version'](binary, '1.2.4', self.scratch)

    @unittest.skipIf(os.name == 'nt', 'the inert binary is a shell script')
    def test_turn_requires_a_delivered_prompt_and_one_success_result(self):
        binary = self.scratch / 'coven-code'
        # Stands in for the engine: forwards stdin to the configured CLI.
        binary.write_text('#!/bin/sh\nexec "$COVEN_CODE_CLAUDE_BIN"\n', encoding='utf-8')
        binary.chmod(0o755)
        SMOKE['check_turn'](binary, self.scratch)
        binary.write_text('#!/bin/sh\ncat > /dev/null\necho \'{"type":"result","is_error":false}\'\n', encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'prompt delivered: False'):
            SMOKE['check_turn'](binary, self.scratch)
        binary.write_text('#!/bin/sh\n"$COVEN_CODE_CLAUDE_BIN" > /dev/null\necho \'{"type":"result","is_error":true}\'\n', encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'stream-json turn failed'):
            SMOKE['check_turn'](binary, self.scratch)


if __name__ == '__main__':
    unittest.main()
