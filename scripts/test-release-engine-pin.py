#!/usr/bin/env python3
"""Hold the release workflow's Coven engine pin request to its contract.

Runs the job's own script against a recording stand-in for `gh`; nothing is
dispatched.
"""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[1]
RELEASE = (ROOT / '.github/workflows/release.yml').read_text(encoding='utf-8')
JOB = RELEASE.split('\n  advance-coven-engine:\n', 1)[1]
STAND_IN = '#!/bin/sh\nprintf \'%s\\n\' "$*" >> "$GH_RECORD"\nexit "${GH_EXIT:-0}"\n'


class CovenEnginePinRequestTests(unittest.TestCase):
    def run_step(self, token, gh_exit=0):
        script = textwrap.dedent(JOB.split('        run: |\n', 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            record = Path(directory) / 'gh-calls'
            stand_in = Path(directory) / 'gh'
            stand_in.write_text(STAND_IN, encoding='utf-8')
            stand_in.chmod(0o700)
            env = {'PATH': f'{directory}:/usr/bin:/bin', 'HOME': directory, 'RELEASE_VERSION': 'v1.2.3',
                   'GH_RECORD': str(record), 'GH_EXIT': str(gh_exit)}
            if token is not None:
                env['GH_TOKEN'] = token
            done = subprocess.run(['bash', '-c', script], cwd=directory, env=env,
                                  capture_output=True, text=True, timeout=10)
            calls = record.read_text(encoding='utf-8').splitlines() if record.exists() else []
            return done, calls

    def test_requests_the_pin_by_default_and_can_be_turned_off(self):
        declared = RELEASE.split('      advance_coven_engine:\n', 1)[1].split('\n\n', 1)[0]
        self.assertIn('        type: boolean\n', declared)
        self.assertTrue(declared.endswith('        default: true'))
        self.assertIn('    if: ${{ inputs.advance_coven_engine }}\n', JOB)

    def test_runs_only_after_publication_and_holds_no_repository_token(self):
        self.assertIn('    needs: release\n', JOB)
        self.assertIn('    permissions: {}\n', JOB)
        self.assertEqual(re.findall(r'secrets\.(\w+)', JOB), ['COVEN_ENGINE_BUMP_TOKEN'])
        self.assertNotIn('actions/checkout', JOB)
        self.assertEqual(RELEASE.count('COVEN_ENGINE_BUMP_TOKEN }}'), 1)

    def test_dispatches_the_engine_bump_workflow_on_coven_main(self):
        done, calls = self.run_step('token')
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertEqual(calls, ['workflow run engine-bump.yml --repo OpenCoven/coven --ref main'])
        self.assertIn('v1.2.3', done.stdout)

    def test_missing_token_warns_and_does_not_fail_the_published_release(self):
        for token in (None, ''):
            with self.subTest(token=token):
                done, calls = self.run_step(token)
                self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
                self.assertEqual(calls, [])
                self.assertIn('::warning::COVEN_ENGINE_BUMP_TOKEN is not set', done.stdout)
                self.assertIn('v1.2.3', done.stdout)

    def test_refused_dispatch_fails_the_job(self):
        done, calls = self.run_step('token', gh_exit=1)
        self.assertNotEqual(done.returncode, 0)
        self.assertEqual(len(calls), 1)
        self.assertNotIn('Asked OpenCoven/coven', done.stdout)


if __name__ == '__main__':
    unittest.main()
