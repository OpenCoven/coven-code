"""Package exactly the five supported engine artifacts and both installers."""
import argparse
from pathlib import Path
import shutil
import tarfile
import zipfile

ARTIFACTS = (
    'coven-code-linux-aarch64',
    'coven-code-linux-x86_64',
    'coven-code-macos-aarch64',
    'coven-code-macos-x86_64',
    'coven-code-windows-x86_64',
)


def package_release(artifacts, output, source):
    entries = {entry.name: entry for entry in artifacts.iterdir()}
    if set(entries) != set(ARTIFACTS):
        raise ValueError('artifact directory must contain exactly the five supported platforms')
    binaries = []
    for name in ARTIFACTS:
        directory = entries[name]
        if directory.is_symlink() or not directory.is_dir():
            raise ValueError(f'{name}: expected a real artifact directory')
        binary_name = 'coven-code.exe' if name.endswith('windows-x86_64') else 'coven-code'
        children = list(directory.iterdir())
        binary = directory / binary_name
        if len(children) != 1 or children[0].name != binary_name:
            raise ValueError(f'{name}: expected only {binary_name}')
        if binary.is_symlink() or not binary.is_file() or binary.stat().st_size == 0:
            raise ValueError(f'{name}: expected a nonempty regular binary')
        binaries.append((name, binary))
    installers = [source / name for name in ('install.sh', 'install.ps1')]
    for installer in installers:
        if installer.is_symlink() or not installer.is_file() or installer.stat().st_size == 0:
            raise ValueError(f'{installer.name}: expected a nonempty regular installer')

    # Never reuse a directory that might contain stale release assets.
    output.mkdir(parents=True, exist_ok=False)
    for name, binary in binaries:
        if binary.suffix == '.exe':
            with zipfile.ZipFile(output / f'{name}.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
                archive.write(binary, arcname=binary.name)
        else:
            with tarfile.open(output / f'{name}.tar.gz', 'w:gz') as archive:
                info = archive.gettarinfo(str(binary), arcname=binary.name)
                info.mode = 0o755
                info.uid = info.gid = 0
                info.uname = info.gname = ''
                with binary.open('rb') as stream:
                    archive.addfile(info, stream)
    for installer in installers:
        shutil.copyfile(installer, output / installer.name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('artifacts', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    try:
        package_release(args.artifacts, args.output, Path(__file__).resolve().parents[1])
    except (OSError, ValueError, tarfile.TarError, zipfile.BadZipFile) as error:
        parser.exit(1, f'error: release packaging failed: {error}\n')


if __name__ == '__main__':
    main()
