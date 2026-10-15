"""Exercise marketplace generation using isolated copies of the CLI and catalog."""

import copy
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class MarketplaceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.prepare_fixture(Path(self.temp.name))

    def prepare_fixture(self, root):
        self.root = root
        (self.root / 'scripts').mkdir()
        (self.root / 'marketplace').mkdir()
        shutil.copyfile(ROOT / 'scripts/sync-marketplace.py', self.root / 'scripts/sync-marketplace.py')
        self.catalog = json.loads((ROOT / 'marketplace/catalog.json').read_text())

    def run_cli(self, *args):
        (self.root / 'marketplace/catalog.json').write_text(json.dumps(self.catalog))
        return subprocess.run(
            [sys.executable, str(self.root / 'scripts/sync-marketplace.py'), *args],
            capture_output=True, text=True, timeout=10,
        )

    def assert_rejected_without_writes(self, diagnostic):
        result = self.run_cli()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(diagnostic, result.stderr)
        self.assertEqual(
            sorted(str(p.relative_to(self.root)) for p in self.root.rglob('*') if p.is_file()),
            ['marketplace/catalog.json', 'scripts/sync-marketplace.py'],
        )

    def test_invalid_package_names(self):
        for name in ['', '.', '..', '../outside', 'ABSOLUTE', 'nested/name',
                     r'nested\name', 'drive:name', 'nul\x00name', None, 17]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as fixture:
                self.prepare_fixture(Path(fixture))
                if name == 'ABSOLUTE':
                    name = str(self.root / 'owned-absolute-fixture')
                self.catalog['plugins'][-1]['name'] = name
                self.assert_rejected_without_writes('Invalid marketplace plugin name')

    def test_duplicate_export_server_reports_both_owners(self):
        first, second = self.catalog['plugins'][:2]
        server = next(iter(first['mcpServers']))
        second['mcpServers'][server] = {'command': 'different-command'}
        self.assert_rejected_without_writes('Duplicate MCP server name')
        result = self.run_cli('--check')
        self.assertEqual(result.returncode, 1)
        self.assertIn(first['name'], result.stderr)
        self.assertIn(second['name'], result.stderr)

    def test_duplicate_plugin_name_is_rejected(self):
        self.catalog['plugins'].append(copy.deepcopy(self.catalog['plugins'][0]))
        self.assert_rejected_without_writes('Duplicate marketplace plugin names')

    def test_generate_check_and_stale_missing_diagnostics(self):
        result = self.run_cli()
        self.assertEqual(result.returncode, 0, result.stderr)
        count = len(self.catalog['plugins'])
        self.assertIn(f'files={3 * count + 4} plugins={count}', result.stdout)
        self.assertEqual(self.run_cli('--check').returncode, 0)
        generated = self.root / 'marketplace/marketplace.json'
        generated.write_text('{}\n')
        result = self.run_cli('--check')
        self.assertEqual(result.returncode, 1)
        self.assertIn('stale marketplace/marketplace.json', result.stderr)
        generated.unlink()
        result = self.run_cli('--check')
        self.assertEqual(result.returncode, 1)
        self.assertIn('missing marketplace/marketplace.json', result.stderr)


if __name__ == '__main__':
    unittest.main()
