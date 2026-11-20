# Coven Code Installation Guide

Coven Code is the Rust coding engine used by the unified Coven CLI. Install
`@opencoven/cli` for the managed engine path. The deprecated
`@opencoven/coven-code` npm package is not published with every engine release.
Direct engine binaries and source builds remain available.

## Managed installation

```bash
npm install -g @opencoven/cli
coven engine install
coven
```

`coven engine install` installs the engine version pinned by the installed Coven
CLI. That version can differ from the latest Coven Code GitHub release.

```bash
coven engine which
coven doctor
```

Use these commands to inspect the selected engine. An explicit `COVEN_ENGINE_BIN`
override takes precedence; otherwise the managed engine takes precedence over a
standalone copy on PATH. See the [unification guide](unification.md).

## Release platforms

| Platform | Architecture | Archive |
|---|---|---|
| Windows | x86_64 | `coven-code-windows-x86_64.zip` |
| Linux | x86_64 | `coven-code-linux-x86_64.tar.gz` |
| Linux | aarch64 | `coven-code-linux-aarch64.tar.gz` |
| macOS | x86_64 | `coven-code-macos-x86_64.tar.gz` |
| macOS | aarch64 | `coven-code-macos-aarch64.tar.gz` |

These are build targets, not a guarantee of compatibility with every OS version.
There is no native Windows ARM64 release artifact.

### Linux runtime requirements

A Linux release binary uses two system libraries and cannot start without them.

- **glibc.** The binary needs at least the glibc of the image it was built on.
  The v0.9.2 archives need glibc 2.39. Ubuntu 24.04 has it. Debian 12 (2.36),
  Ubuntu 22.04 (2.35), RHEL 9 and its rebuilds (2.34) and Amazon Linux 2023
  (2.34) do not. A later release can need a newer one.
  `getconf GNU_LIBC_VERSION` prints the system's version.
- **ALSA.** `libasound.so.2` must be installed, even on a server that never uses
  voice. On Debian and Ubuntu the package is `libasound2t64` or `libasound2`; on
  Fedora and RHEL it is `alsa-lib`. Minimal container images usually lack it.

On Linux, the shell installer runs the binary before it installs it. If the binary cannot
start, the installer prints the loader's message, says which requirement is
missing, installs nothing and leaves an existing install in place.
`coven engine install` does not make this check; `coven doctor` reports whether
the engine it installed can run.

On a system with an older glibc, [build from source](#building-from-source) on that
system.

## Standalone installation

Download the installer from the chosen
[GitHub Release](https://github.com/OpenCoven/coven-code/releases), inspect it,
and run it locally. Use the installer belonging to that release when pinning a
version. Launch the installed engine with `coven-code`; the installers also create
`coven-cave` as an alias. They do not install the unified `coven` CLI.

The default location is `~/.coven-code/bin/` on Unix and
`%USERPROFILE%\.coven-code\bin\` on Windows. The shell installer appends a PATH
entry to a recognized shell's configuration; the PowerShell installer updates
the user PATH unless `-NoModifyPath` is supplied. Reopen the terminal afterward.
The shell installer does not remove macOS quarantine attributes automatically.

| Shell flag | PowerShell parameter | Effect |
|---|---|---|
| `--version <version>` | `-Version <version>` | Select a release; use its numeric version without `v` |
| `--binary <path>` | `-Binary <path>` | Install an already verified local binary |
| `--install-dir <path>` | `-InstallDir <path>` | Override the install directory |
| Not supported | `-NoModifyPath` | Leave the Windows user PATH unchanged |
| `--force` | `-Force` | Replace an existing engine that differs from this one without asking |
| `--help` | `-Help` | Print usage |

An installer replaces an existing `coven-code` in the install directory only
with consent. It installs without asking when the existing file has the same
bytes as the new one, or when a download is for the version the existing engine
reports as `coven-code <version>`. Otherwise it asks `Replace it? [y/N]` if its
input is a terminal, and replaces the engine only on `y` or `yes`;
`irm ... | iex` in a console asks, `curl ... | bash` never does. With no
terminal, as under `curl ... | bash` or in CI, it leaves the existing engine and
alias untouched and fails: the shell installer exits non-zero, and the
PowerShell installer throws. Upgrading an older standalone install with a piped
one-liner therefore needs consent without a prompt: `--force` (`-Force`), or
`COVEN_CODE_INSTALL_FORCE=1` (exactly `1`) in the environment:

```bash
curl -fsSL https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh | bash -s -- --force
```

```powershell
$env:COVEN_CODE_INSTALL_FORCE = '1'; try { irm https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 | iex } finally { Remove-Item Env:COVEN_CODE_INSTALL_FORCE }
```

The check covers `coven-code`, not the `coven-cave` alias beside it.

The local-binary path does not authenticate the file for you. Verify its origin
and digest before installing it. The shell installer may still resolve the
latest release if no `--version` is supplied, even with `--binary`.

### Windows PowerShell 5.1: 'Get-FileHash' is not recognized

`install.ps1` runs under PowerShell 7 (`pwsh`) and under Windows PowerShell 5.1
(`powershell`). Typing `powershell` at a PowerShell 7 prompt is fine: PowerShell
7 removes its own module paths from `PSModulePath` for a Windows PowerShell it
starts directly. A Windows PowerShell started through another program that
PowerShell 7 launched, such as `cmd.exe`, Python, a build tool or an editor
task, inherits PowerShell 7's `PSModulePath` unchanged. It then finds PowerShell
7's copies of built-in modules first, cannot load them, and cannot auto-load some
commands, including `Get-FileHash`. See
[about_PSModulePath](https://learn.microsoft.com/powershell/module/microsoft.powershell.core/about/about_psmodulepath).

The installer then fails at the checksum step with output such as:

```
Download failed: The term 'Get-FileHash' is not recognized as the name of a cmdlet, ...
Check that release v<version> exists for windows-x86_64:
  https://github.com/OpenCoven/coven-code/releases/tag/v<version>
```

The release exists; the shell is the problem. The installer stops before it
extracts the archive: no executable is installed, an existing install is left
as it was, and the user PATH is not changed (the install directory may be left
behind, empty). Run it in one of these ways instead:

- Run it under PowerShell 7: `pwsh -NoProfile -File .\install.ps1`.
- Start Windows PowerShell 5.1 from the Start menu, or from a Command Prompt
  that was not started from PowerShell 7, directly or through another program,
  and run it there.
- Clear the variable for the Windows PowerShell you start, so it builds its own
  module path. This works from a Command Prompt or PowerShell, run in the folder
  that holds `install.ps1` (not a `\\server\share` path):

  ```
  cmd /c "set PSModulePath=&& powershell -NoProfile -File .\install.ps1"
  ```

Pass the installer's parameters after `.\install.ps1`. If running scripts is
disabled, and you have inspected the script, put `-ExecutionPolicy Bypass`
before `-File` (for example `pwsh -NoProfile -ExecutionPolicy Bypass -File
.\install.ps1`); at an interactive prompt, run
`Set-ExecutionPolicy -Scope Process Bypass` first. Installs with `-Binary` do
not use `Get-FileHash` and are not affected, but check that binary's digest in
a shell that can load `Get-FileHash`, such as PowerShell 7, or with
`certutil -hashfile <file> SHA256`.

### Archive verification

For releases with `SHA256SUMS`, obtain the archive and manifest from the same
versioned release. Verify the selected archive against its exact manifest entry
before extracting or replacing an executable. Every archive should contain only
`coven-code` (or `coven-code.exe` on Windows).

The next-release checksum-verifying installers require a complete manifest for
all five archives and fail before extraction on missing, malformed or mismatched
checksums. Older releases, including v0.8.0, lack that manifest and cannot be
installed with those new download paths. Historical installers do not provide
this verification. A checksum delivered over the same HTTPS release channel is
not an independently signed build attestation.

For manual installation, use the verified archive, inspect its contents, then
place the extracted executable in a directory on PATH. Do not pipe an unverified
download directly into an archive extractor.

## Upgrading and rollback

For a managed installation:

```bash
npm install -g @opencoven/cli@latest
coven engine install
coven engine which
coven doctor
```

For a standalone installation, use the selected release's verified installer or
archive, then check the exact installed path and version. Updating a standalone
binary does not override a managed engine.

Before upgrading, preserve a verified prior binary and back up the affected
engine configuration and session data. Standalone settings normally live under
`~/.coven-code/`; under Coven they normally live under `~/.coven/code/`.
`COVEN_CODE_HOME` can override the engine home, and `COVEN_HOME` can relocate the
Coven home. Inspect the effective paths, including migration symlinks, rather
than assuming the default directory is active.

Rollback requires a verified prior binary and data compatible with that version.
Do not disable checksum verification to install a manifest-less historical
release. Restore an independently verified artifact and, if required, the
matching data backup. Consult the release notes for migration-specific limits;
a successful binary replacement alone does not prove data rollback safety.

## Verify the installation

```bash
coven-code --version
```

Example engine output:

```
coven-code 0.9.2
```

Use `command -v coven-code` on Unix or `Get-Command coven-code` in PowerShell to
inspect the standalone executable. Use `coven engine which` for the managed
selection; the two paths may differ.

## Building from source

Use the Rust toolchain CI and release builds use, and the checked-in lockfile.
That toolchain is Rust 1.100.0, pinned in the `Install Rust toolchain` steps
under `.github/workflows/`. No older minimum Rust version is asserted here
without a corresponding build check.

```bash
git clone https://github.com/OpenCoven/coven-code.git
cd coven-code/src-rust
cargo build --locked --package claurst
```

The debug binary is `target/debug/coven-code` (`.exe` on Windows). For an optimized
binary, use `cargo build --release --locked --package claurst`; output is under
`target/release/`. To install a source build into Cargo's bin directory:

```bash
cargo install --path crates/cli --locked
```

The package has one binary target, `coven-code`; standalone release installers
create the `coven-cave` alias separately. The CLI's default feature is `voice`.
Use `--no-default-features` to disable it, or `--features steer` for the opt-in
steering command. Do not pass features that exist only on internal crates to
the CLI package.

On Debian/Ubuntu the release workflow installs `libasound2-dev` and `pkg-config`.
ALSA headers are needed for the voice feature. Use the repository's release
workflow for Linux aarch64 cross-builds: its Cross.toml pre-build setup installs
the target ALSA headers and pkg-config inside the container.

To upgrade a source install, update the checkout to the intended reviewed source,
then run `cargo install --path crates/cli --locked --force` from `src-rust/`.
Use `coven-code --help` for supported CLI flags; no generated shell-completion
command is currently documented.

## Uninstalling

Remove the executable and installer-created alias from the actual installation
location, then remove only the corresponding PATH entry. For an old npm install,
use `npm uninstall -g @opencoven/coven-code` or
`bun remove -g @opencoven/coven-code`. For a Cargo install, use `cargo uninstall claurst`.

Keep configuration and sessions unless you intentionally want to delete them.
Do not delete all of `~/.coven`: it contains shared ecosystem state. If removing
engine data, first identify the effective engine home, inspect symlinks, and back
it up. Remove only the engine-owned data you have selected; do not follow a
migration alias into unrelated state.
