"""Adversarial subprocess coverage for the reverse-contract CI guard."""
import contextlib
import io
import json
from pathlib import Path
import runpy
import sys
import tempfile
import unittest

RUNNER = runpy.run_path(str(Path(__file__).with_name('run-coven-contract.py')))


class ContractRunner(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='contract-guard-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.engine = self.root / 'engine'
        self.engine.write_text('#!/bin/sh\nexit 0\n')
        self.engine.chmod(0o700)
        self.binary = self.root / 'consumer'

    def fixture(self, mode):
        # This fixture emulates libtest output, checks isolation, and actually
        # starts the supplied engine. Real consumer coverage is a separate gate.
        self.binary.write_text(f'''#!{sys.executable}
import os, subprocess, sys, time
mode = {mode!r}
names = {RUNNER['TESTS']!r}
assert 'ANTHROPIC_API_KEY' not in os.environ
assert os.path.isabs(os.environ['COVEN_ENGINE_BIN'])
assert os.path.isdir(os.environ['COVEN_CODE_HOME'])
if '--list' in sys.argv:
    for name in (names[:-1] if mode == 'missing' else names):
        print(name + ': test')
    sys.exit(0)
if mode == 'timeout':
    subprocess.Popen(['/bin/sleep', '30'])
    time.sleep(30)
if mode == 'failure': sys.exit(101)
if mode == 'skip': print('contract: skipped (COVEN_ENGINE_BIN unset)')
subprocess.run([os.environ['COVEN_ENGINE_BIN']], check=True)
count = 0 if mode == 'empty' else 1
ignored = 1 if mode == 'ignored' else 0
print(f'test result: ok. {{count}} passed; 0 failed; {{ignored}} ignored; 0 measured; 99 filtered out; finished in 0.00s')
''')
        self.binary.chmod(0o700)

    def test_success_and_isolation(self):
        self.fixture('success')
        with contextlib.redirect_stdout(io.StringIO()):
            RUNNER['verify'](self.engine, self.binary, 5)

    def test_rejects_false_success_and_failure(self):
        for mode in ('missing', 'skip', 'empty', 'ignored', 'failure', 'timeout'):
            with self.subTest(mode=mode):
                self.fixture(mode)
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                    RUNNER['verify'](self.engine, self.binary, 0.5 if mode == 'timeout' else 5)

    def test_missing_engine(self):
        self.fixture('success')
        self.engine.unlink()
        with self.assertRaises(ValueError):
            RUNNER['verify'](self.engine, self.binary, 5)

    def test_cargo_artifact_selection(self):
        self.fixture('success')
        artifacts = self.root / 'cargo.jsonl'
        record = {'reason': 'compiler-artifact', 'target': {'name': 'coven', 'kind': ['bin']},
                  'profile': {'test': True}, 'executable': str(self.binary)}
        artifacts.write_text(json.dumps({'reason': 'build-finished', 'success': True}) + '\n'
                             + json.dumps(record) + '\n')
        self.assertEqual(RUNNER['consumer_executable'](artifacts), self.binary.resolve())
        record['profile']['test'] = False
        artifacts.write_text(json.dumps(record) + '\n')
        with self.assertRaises(ValueError):
            RUNNER['consumer_executable'](artifacts)
        record['profile']['test'] = True
        duplicate = dict(record, executable=str(self.engine))
        artifacts.write_text(json.dumps(record) + '\n' + json.dumps(duplicate) + '\n')
        with self.assertRaises(ValueError):
            RUNNER['consumer_executable'](artifacts)


if __name__ == '__main__':
    unittest.main()
