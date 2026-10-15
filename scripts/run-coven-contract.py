"""Run the consumer's exact engine tests, rejecting empty or skipped success.

Build coven-cli with cargo test --no-run --message-format=json first, then pass
that stdout file as --cargo-artifacts. Runtime state and credentials are isolated.
This checks the consumer's narrow invocation contract, not hosted conformance.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile

TESTS = (
    'tests::contract_version_output_parses',
    'tests::contract_auth_status_json_is_machine_readable',
    'tests::contract_print_flags_are_accepted',
)


def consumer_executable(artifacts):
    binaries = set()
    for line in artifacts.read_text().splitlines():
        event = json.loads(line)
        if (event.get('reason') == 'compiler-artifact'
                and event.get('target', {}).get('name') == 'coven'
                and 'bin' in event.get('target', {}).get('kind', [])
                and event.get('profile', {}).get('test') is True
                and event.get('executable')):
            binaries.add(Path(event['executable']).resolve())
    if len(binaries) != 1:
        raise ValueError(f'Expected one coven test executable, found {len(binaries)}')
    binary = binaries.pop()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError(f'Consumer test executable is missing or not executable: {binary}')
    return binary


def run(command, env, cwd, timeout):
    # A test may wait on an engine child. Kill the entire owned process group on
    # timeout, not just libtest, so CI cannot leave an orphan engine running.
    with subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True,
                          start_new_session=True) as child:
        try:
            output, _ = child.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            output, _ = child.communicate()
            print(output, end='', flush=True)
            raise ValueError(f'Contract command exceeded {timeout}s: {command}') from None
        print(output, end='', flush=True)
        if child.returncode != 0:
            raise ValueError(f'Contract command failed ({child.returncode}): {command}')
        if 'contract: skipped' in output.lower():
            raise ValueError('Consumer skipped engine contract execution')
        return output


def verify(engine, binary, timeout):
    engine = engine.resolve()
    if not engine.is_file() or not os.access(engine, os.X_OK):
        raise ValueError(f'Engine is missing or not executable: {engine}')
    with tempfile.TemporaryDirectory(prefix='coven-contract-') as temporary:
        state = Path(temporary)
        for name in ('code', 'coven', 'project'):
            (state / name).mkdir()
        (state / 'code/settings.json').write_text('{"hasCompletedOnboarding":true}\n')
        env = {
            'PATH': '/usr/bin:/bin:/usr/sbin:/sbin',
            'SHELL': '/bin/sh', 'TERM': 'dumb', 'LANG': 'C',
            'TMPDIR': temporary,
            'COVEN_CODE_HOME': str(state / 'code'),
            'COVEN_HOME': str(state / 'coven'),
            'COVEN_DAEMON_SOCKET': str(state / 'absent.sock'),
            'COVEN_CODE_DISABLE_MODELS_FETCH': '1',
            'COVEN_CODE_SKIP_PROMPT_HISTORY': '1',
            'COVEN_PARENT': 'coven',
            'COVEN_ENGINE_BIN': str(engine),
        }
        for key in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY'):
            env[key] = env[key.lower()] = 'http://127.0.0.1:9'
        env['NO_PROXY'] = env['no_proxy'] = ''
        for name in ('claude', 'codex'):
            if shutil.which(name, path=env['PATH']):
                raise ValueError(f'Provider CLI unexpectedly available in test PATH: {name}')
        cwd = state / 'project'
        listing = run([str(binary), '--list', '--exact', *TESTS], env, cwd, timeout)
        discovered = re.findall(r'^(.+): test$', listing, re.MULTILINE)
        if sorted(discovered) != sorted(TESTS):
            raise ValueError(f'Expected exact engine contract tests, found {discovered!r}')
        for test in TESTS:
            output = run([str(binary), '--exact', test, '--nocapture',
                          '--test-threads=1'], env, cwd, timeout)
            if not re.search(r'^test result: ok\. 1 passed; 0 failed; 0 ignored; '
                             r'0 measured; \d+ filtered out;', output, re.MULTILINE):
                raise ValueError(f'Expected one executed passing test: {test}')
        print('Verified 3 executed consumer engine contract tests.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--cargo-artifacts', type=Path, required=True)
    parser.add_argument('--timeout', type=float, default=90)
    args = parser.parse_args()
    if not 0 < args.timeout <= 600:
        parser.error('--timeout must be between 0 and 600 seconds')
    try:
        verify(args.engine, consumer_executable(args.cargo_artifacts), args.timeout)
    except (ValueError, OSError) as error:
        print(f'error: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
