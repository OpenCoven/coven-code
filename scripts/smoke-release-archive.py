"""Verify one packaged release archive and run its binary on this machine.

Reads the archive's entry in SHA256SUMS, checks the digest and the archive's
single member, then runs the extracted binary: `--version` must print the
version stamped in the source tree, and one stream-json turn must complete
against a recording stand-in for the Claude CLI. No model is called and no
credentials are read.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile

ARTIFACTS = (
    'coven-code-linux-aarch64',
    'coven-code-linux-x86_64',
    'coven-code-macos-aarch64',
    'coven-code-macos-x86_64',
    'coven-code-windows-x86_64',
)
PROMPT = 'release archive smoke prompt'
STAND_IN = '''import sys
with open({record!r}, 'w', encoding='utf-8') as record:
    record.write(sys.stdin.read())
for frame in (
    '{{"type":"system","subtype":"init","session_id":"smoke"}}',
    '{{"type":"assistant","message":{{"content":[{{"type":"text","text":"accepted"}}]}}}}',
    '{{"type":"result","subtype":"success","is_error":false,"usage":{{"input_tokens":1,"output_tokens":1}}}}',
):
    print(frame, flush=True)
'''


def stamped_version(root):
    workspace = tomllib.loads((root / 'src-rust/Cargo.toml').read_text(encoding='utf-8'))
    return workspace['workspace']['package']['version']


def archive_name(artifact):
    if artifact not in ARTIFACTS:
        raise ValueError(f'{artifact}: not one of the five supported artifacts')
    return f'{artifact}.zip' if artifact.endswith('windows-x86_64') else f'{artifact}.tar.gz'


def verify_archive(release, artifact, destination):
    """Check the manifest digest and the single member, then extract it."""
    name = archive_name(artifact)
    entries = {}
    for line in (release / 'SHA256SUMS').read_text(encoding='utf-8').splitlines():
        digest, separator, entry = line.partition('  ')
        if not separator or entry in entries:
            raise ValueError('SHA256SUMS: malformed or repeated entry')
        entries[entry] = digest
    if name not in entries:
        raise ValueError(f'SHA256SUMS: no entry for {name}')
    actual = hashlib.sha256((release / name).read_bytes()).hexdigest()
    if actual != entries[name]:
        raise ValueError(f'{name}: digest {actual} does not match the manifest')
    binary_name = 'coven-code.exe' if name.endswith('.zip') else 'coven-code'
    if name.endswith('.zip'):
        with zipfile.ZipFile(release / name) as archive:
            if archive.namelist() != [binary_name]:
                raise ValueError(f'{name}: expected only {binary_name}')
            payload = archive.read(binary_name)
    else:
        with tarfile.open(release / name) as archive:
            members = archive.getmembers()
            if [member.name for member in members] != [binary_name] or not members[0].isfile():
                raise ValueError(f'{name}: expected only a regular {binary_name}')
            payload = archive.extractfile(members[0]).read()
    if not payload:
        raise ValueError(f'{name}: {binary_name} is empty')
    binary = destination / binary_name
    binary.write_bytes(payload)
    binary.chmod(0o755)
    return binary


def scratch_environment(scratch):
    """A home with no settings or credentials, and no reachable network proxy."""
    environment = {
        name: os.environ[name]
        for name in ('PATH', 'PATHEXT', 'SystemRoot', 'SYSTEMROOT', 'windir', 'ComSpec', 'TEMP', 'TMP')
        if name in os.environ
    }
    for name in ('HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'COVEN_HOME'):
        directory = scratch / name.lower()
        directory.mkdir(exist_ok=True)
        environment[name] = str(directory)
    environment.update(USER='release-smoke', HTTP_PROXY='http://127.0.0.1:1', HTTPS_PROXY='http://127.0.0.1:1')
    return environment


def check_version(binary, version, scratch):
    done = subprocess.run([str(binary), '--version'], capture_output=True, text=True, timeout=60,
                          env=scratch_environment(scratch), cwd=scratch)
    printed = done.stdout.strip()
    if done.returncode != 0 or printed != f'coven-code {version}':
        raise ValueError(f'--version: expected "coven-code {version}", got {printed!r} '
                         f'(exit {done.returncode}) {done.stderr.strip()[-400:]}')
    return printed


def check_turn(binary, scratch):
    """Run one stream-json turn through the query loop and the keyless adapter."""
    record = scratch / 'provider-input.json'
    record.unlink(missing_ok=True)
    script = scratch / 'stand_in.py'
    script.write_text(STAND_IN.format(record=str(record)), encoding='utf-8')
    if os.name == 'nt':
        launcher = scratch / 'claude.cmd'
        launcher.write_text(f'@"{sys.executable}" "{script}" %*\r\n', encoding='utf-8')
    else:
        launcher = scratch / 'claude'
        launcher.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{script}" "$@"\n', encoding='utf-8')
        launcher.chmod(0o755)
    environment = scratch_environment(scratch)
    environment['COVEN_CODE_CLAUDE_BIN'] = str(launcher)
    frame = json.dumps({'type': 'user', 'message': {'role': 'user', 'content': PROMPT}}) + '\n'
    done = subprocess.run(
        [str(binary), '--print', '--input-format', 'stream-json', '--output-format', 'stream-json'],
        input=frame, capture_output=True, text=True, encoding='utf-8', timeout=180,
        env=environment, cwd=scratch)
    results = []
    for line in done.stdout.splitlines():
        try:
            parsed = json.loads(line)
        except ValueError:
            continue
        if isinstance(parsed, dict) and parsed.get('type') == 'result':
            results.append(parsed)
    delivered = record.read_text(encoding='utf-8') if record.exists() else ''
    if (done.returncode != 0 or len(results) != 1 or results[0].get('is_error') is not False
            or PROMPT not in delivered):
        raise ValueError(f'stream-json turn failed: exit {done.returncode}, results {results}, '
                         f'prompt delivered: {PROMPT in delivered}, stderr: {done.stderr.strip()[-800:]}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('release', type=Path, help='directory holding the archives and SHA256SUMS')
    parser.add_argument('artifact', choices=ARTIFACTS)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        version = stamped_version(root)
        with tempfile.TemporaryDirectory(prefix='release-smoke-') as temporary:
            scratch = Path(temporary).resolve()
            binary = verify_archive(args.release, args.artifact, scratch)
            print(f'{archive_name(args.artifact)}: digest and contents verified')
            print(f'{check_version(binary, version, scratch)}: version matches the source stamp')
            check_turn(binary, scratch)
            print('stream-json turn completed against the recording stand-in')
    except (OSError, ValueError, KeyError, subprocess.SubprocessError,
            tarfile.TarError, zipfile.BadZipFile) as error:
        parser.exit(1, f'error: release archive smoke failed: {error}\n')


if __name__ == '__main__':
    main()
