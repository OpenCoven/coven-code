"""Read-only release preflight: forward SemVer and consistent version stamps."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib

SEMVER = re.compile(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)'
                    r'(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?'
                    r'(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?')


def precedence(tag):
    match = SEMVER.fullmatch(tag)
    if not match:
        raise ValueError(f'Invalid SemVer tag: {tag}')
    identifiers = []
    for part in (match[4].split('.') if match[4] else []):
        if part.isdigit():
            if len(part) > 1 and part.startswith('0'):
                raise ValueError(f'Leading zero in prerelease identifier: {tag}')
            identifiers.append((0, int(part)))
        else:
            identifiers.append((1, part))
    return (*(int(match[i]) for i in (1, 2, 3)), not bool(match[4]), tuple(identifiers))


def check_forward(tag, tags):
    requested = precedence(tag)
    for existing in tags:
        # Ignore unrelated tags, but fail on malformed version-shaped tags:
        # silently ignoring one could defeat the forward-only guarantee.
        if re.match(r'^v[0-9]', existing) and requested <= precedence(existing):
            raise ValueError(f'{tag} must be strictly newer than {existing}')


def check_release(root, tag, tags):
    check_forward(tag, tags)
    version = tag[1:]
    workspace = tomllib.loads((root / 'src-rust/Cargo.toml').read_text())
    versions = {'Cargo workspace': workspace['workspace']['package']['version']}
    packages = tomllib.loads((root / 'src-rust/Cargo.lock').read_text())['package']
    expected = set()
    for member in workspace['workspace']['members']:
        manifest = tomllib.loads((root / 'src-rust' / member / 'Cargo.toml').read_text())
        package = manifest['package']
        expected.add(package['name'])
        if package['version'] != {'workspace': True}:
            raise ValueError(f'{member} does not inherit the workspace version')
    actual = [p for p in packages if p['name'] in expected and 'source' not in p]
    if len(actual) != len(expected) or {p['name'] for p in actual} != expected:
        raise ValueError('Cargo.lock workspace package set does not match workspace members')
    versions.update({f"Cargo.lock {p['name']}": p['version'] for p in actual})
    versions['npm'] = json.loads((root / 'npm/package.json').read_text())['version']
    agent = json.loads((root / 'src-rust/crates/acp/registry-template/agent.json').read_text())
    versions['ACP registry'] = agent['version']
    for label, value in versions.items():
        if value != version:
            raise ValueError(f'{label}: expected {version}, found {value}')
    archives = {
        'darwin-aarch64': 'coven-code-macos-aarch64.tar.gz',
        'darwin-x86_64': 'coven-code-macos-x86_64.tar.gz',
        'linux-aarch64': 'coven-code-linux-aarch64.tar.gz',
        'linux-x86_64': 'coven-code-linux-x86_64.tar.gz',
        'windows-x86_64': 'coven-code-windows-x86_64.zip',
    }
    binaries = agent['distribution']['binary']
    if set(binaries) != set(archives):
        raise ValueError('ACP registry must contain the five supported platforms')
    for platform, archive in archives.items():
        expected_url = f'https://github.com/OpenCoven/coven-code/releases/download/{tag}/{archive}'
        if binaries[platform]['archive'] != expected_url:
            raise ValueError(f'ACP archive URL mismatch for {platform}')
    for name, patterns in {
        'README.md': [rf'Beta \(v{re.escape(version)}\)\.', rf'`coven-code` \*\*v{re.escape(version)}\*\*'],
        'docs/index.md': [rf'^\*\*Version:\*\* {re.escape(version)}(?=\s|$)'],
        'docs/installation.md': [rf'^coven-code {re.escape(version)}$'],
    }.items():
        text = (root / name).read_text()
        if not all(re.search(pattern, text, re.MULTILINE) for pattern in patterns):
            raise ValueError(f'{name}: version stamp missing or inconsistent')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('tag')
    # Auto-release checks ordering before it runs bump-version.py, when the
    # canonical stamps do not exist yet; it must not verify them.
    parser.add_argument('--forward-only', action='store_true',
                        help='check SemVer ordering against origin tags only')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        # Query the authoritative remote rather than relying on stale local tags.
        refs = subprocess.run(['git', 'ls-remote', '--tags', 'origin'], cwd=root,
                              check=True, capture_output=True, text=True, timeout=60).stdout
        tags = {line.split('\t', 1)[1].removeprefix('refs/tags/').removesuffix('^{}')
                for line in refs.splitlines()}
        if args.forward_only:
            check_forward(args.tag, tags)
        else:
            check_release(root, args.tag, tags)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        print(f'error: release preflight failed: {error}', file=sys.stderr)
        return 1
    if args.forward_only:
        print(f'{args.tag}: forward version verified against origin tags')
    else:
        print(f'{args.tag}: forward version and all canonical stamps verified')
    return 0


if __name__ == '__main__':
    sys.exit(main())
