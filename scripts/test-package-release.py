"""Exercise real archives and rejection paths using inert binary fixtures."""
from pathlib import Path
import runpy
import tarfile
import tempfile
import unittest
import zipfile

PACK = runpy.run_path(str(Path(__file__).with_name('package-release.py')))


class ReleasePackaging(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='release-package-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.artifacts = self.root / 'artifacts'
        self.output = self.root / 'release'
        for name in PACK['ARTIFACTS']:
            directory = self.artifacts / name
            directory.mkdir(parents=True)
            binary = 'coven-code.exe' if name.endswith('windows-x86_64') else 'coven-code'
            (directory / binary).write_bytes(f'inert:{name}'.encode())
        for name in ('install.sh', 'install.ps1'):
            (self.root / name).write_text(f'inert:{name}')

    def package(self):
        PACK['package_release'](self.artifacts, self.output, self.root)

    def test_exact_members_bytes_and_unix_mode(self):
        self.package()
        self.assertEqual(len(list(self.output.iterdir())), 7)
        for name in PACK['ARTIFACTS']:
            expected = f'inert:{name}'.encode()
            if name.endswith('windows-x86_64'):
                with zipfile.ZipFile(self.output / f'{name}.zip') as archive:
                    self.assertEqual(archive.namelist(), ['coven-code.exe'])
                    self.assertEqual(archive.read('coven-code.exe'), expected)
            else:
                with tarfile.open(self.output / f'{name}.tar.gz') as archive:
                    self.assertEqual(archive.getnames(), ['coven-code'])
                    member = archive.getmember('coven-code')
                    self.assertTrue(member.isfile())
                    self.assertEqual(member.mode, 0o755)
                    self.assertEqual(archive.extractfile(member).read(), expected)
        for name in ('install.sh', 'install.ps1'):
            self.assertEqual((self.output / name).read_bytes(), (self.root / name).read_bytes())

    def test_wrong_or_extra_member_rejects_before_output(self):
        directory = self.artifacts / PACK['ARTIFACTS'][0]
        binary = directory / 'coven-code'
        binary.rename(directory / 'README.txt')
        with self.assertRaisesRegex(ValueError, 'expected only'):
            self.package()
        (directory / 'README.txt').rename(binary)
        (directory / 'extra').write_text('extra')
        with self.assertRaisesRegex(ValueError, 'expected only'):
            self.package()
        self.assertFalse(self.output.exists())

    def test_missing_or_extra_platform_rejects(self):
        directory = self.artifacts / PACK['ARTIFACTS'][0]
        directory.rename(self.root / 'held')
        with self.assertRaisesRegex(ValueError, 'exactly the five'):
            self.package()
        (self.root / 'held').rename(directory)
        (self.artifacts / 'unexpected').mkdir()
        with self.assertRaisesRegex(ValueError, 'exactly the five'):
            self.package()
        self.assertFalse(self.output.exists())

    def test_empty_binary_and_directory_reject(self):
        binary = self.artifacts / PACK['ARTIFACTS'][0] / 'coven-code'
        binary.write_bytes(b'')
        with self.assertRaisesRegex(ValueError, 'nonempty regular'):
            self.package()
        binary.unlink()
        binary.mkdir()
        with self.assertRaisesRegex(ValueError, 'nonempty regular'):
            self.package()
        self.assertFalse(self.output.exists())

    def test_missing_installer_rejects(self):
        (self.root / 'install.ps1').unlink()
        with self.assertRaisesRegex(ValueError, 'install.ps1'):
            self.package()
        self.assertFalse(self.output.exists())

    def test_stale_output_is_preserved_and_rejected(self):
        self.output.mkdir()
        sentinel = self.output / 'old'
        sentinel.write_text('preserve')
        with self.assertRaises(FileExistsError):
            self.package()
        self.assertEqual(sentinel.read_text(), 'preserve')

    def test_symlink_binary_rejects(self):
        binary = self.artifacts / PACK['ARTIFACTS'][0] / 'coven-code'
        binary.unlink()
        try:
            binary.symlink_to(self.root / 'install.sh')
        except OSError as error:
            self.skipTest(f'symlink creation unavailable: {error}')
        with self.assertRaisesRegex(ValueError, 'nonempty regular'):
            self.package()
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
