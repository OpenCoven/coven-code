# Coven Code Installation Guide

Coven Code is a Rust reimplementation of the Claude Code CLI. The fastest way
to install it is via the one-liner installers below. They drop the binary
into `~/.coven-code/bin` (or `%USERPROFILE%\.coven-code\bin` on Windows) and add
that directory to your `PATH` automatically.

---

## System Requirements

| Platform | Architecture | Minimum OS |
|----------|-------------|------------|
| Windows  | x86_64      | Windows 10 / Server 2019 |
| Linux    | x86_64      | glibc 2.17+ (most distros from 2014 onward) |
| Linux    | aarch64     | glibc 2.17+ (Raspberry Pi 4, AWS Graviton, etc.) |
| macOS    | x86_64      | macOS 11 Big Sur |
| macOS    | aarch64     | macOS 11 Big Sur (Apple Silicon: M1/M2/M3) |

There are no other runtime dependencies. The binary is statically linked where
possible; on Linux it links against the system glibc.

---

## Quick install (recommended)

### Linux / macOS

```bash
curl -fsSL https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh | bash
```

### Windows (PowerShell)

```powershell
irm https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 | iex
```

Both installers:

1. Detect your platform and architecture.
2. Download the matching archive from the latest GitHub release.
3. Extract `coven-code` into `~/.coven-code/bin/` (Windows: `%USERPROFILE%\.coven-code\bin\`).
4. Append that directory to your shell config (`.bashrc`, `.zshrc`,
   `.config/fish/config.fish`) or to your Windows user `PATH`.
5. On macOS, strip the quarantine attribute so Gatekeeper does not block the
   unsigned binary.

Open a new terminal afterwards (or `source` the modified shell config) so
the updated `PATH` takes effect, then run `coven-code --version` to verify.

### Installer flags

Both scripts accept the same flags:

| Flag (sh) | Flag (ps1) | Effect |
|---|---|---|
| `--version 0.1.0` | `-Version 0.1.0` | Install a specific version |
| `--binary <path>` | `-Binary <path>` | Install from a local file (skip download) |
| `--install-dir <path>` | `-InstallDir <path>` | Override the install directory |
| `--no-modify-path` | `-NoModifyPath` | Don't touch shell config / user PATH |
| `--help` | `-Help` | Show usage |

Example: `curl -fsSL https://.../install.sh | bash -s -- --version 0.1.0`

---

## Via npm / bun

If you have Node.js or Bun installed, you can install Coven Code as a global
package. The postinstall script automatically downloads the correct pre-built
native binary for your platform from GitHub Releases — no compilation needed.

```bash
# npm
npm install -g coven-code

# bun
bun install -g coven-code
```

After installation, run `coven-code` directly from your terminal.

You can also run Coven Code without a permanent install:

```bash
npx coven-code          # via npm
bunx coven-code         # via bun
```

**Supported platforms via npm:**

| Platform | Architecture |
|----------|-------------|
| Linux    | x86_64, aarch64 |
| macOS    | x86_64 (Intel), aarch64 (Apple Silicon) |
| Windows  | x86_64 |

---

## Upgrading

Once installed, upgrade in place at any time:

```bash
coven-code upgrade               # to the latest release
coven-code upgrade --version 0.1.0   # pin to a specific version
coven-code upgrade --force       # reinstall the same version
```

The upgrade command downloads the matching archive from GitHub, extracts the
new binary, and replaces the running executable atomically. Settings in
`~/.coven-code/` are preserved.

---

## Manual install from GitHub Releases

If you'd rather not run an install script, grab archives directly from
[**GitHub Releases**](https://github.com/OpenCoven/coven-code/releases):

| Archive | Platform |
|---------|----------|
| `coven-code-windows-x86_64.zip` | Windows 64-bit |
| `coven-code-linux-x86_64.tar.gz` | Linux x86_64 |
| `coven-code-linux-aarch64.tar.gz` | Linux ARM64 |
| `coven-code-macos-x86_64.tar.gz` | macOS Intel |
| `coven-code-macos-aarch64.tar.gz` | macOS Apple Silicon |

Every archive contains a single binary named `coven-code` (or `coven-code.exe`).
Extract it and put it somewhere on your `PATH`. For example on Linux:

```bash
curl -L https://github.com/OpenCoven/coven-code/releases/latest/download/coven-code-linux-x86_64.tar.gz \
  | tar -xz
chmod +x coven-code
sudo mv coven-code /usr/local/bin/
```

On macOS, also strip the quarantine flag so Gatekeeper allows the unsigned
binary:

```bash
xattr -rd com.apple.quarantine /usr/local/bin/coven-code
```

On Windows, extract the zip and add the folder containing `coven-code.exe`
to your user `PATH` via **Settings → System → Advanced system settings →
Environment Variables**.

### User-local install without sudo

```bash
mkdir -p ~/.local/bin
mv coven-code ~/.local/bin/coven-code
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

For Zsh users, substitute `.zshrc` for `.bashrc`.

---

## Verifying the Installation

```bash
coven-code --version
```

A successful installation prints the version string, for example:

```
coven-code 0.0.13
```

To confirm the binary is the one you installed:

```bash
which coven-code          # Linux / macOS
where coven-code          # Windows (Command Prompt)
```

---

## Building from Source

Building from source requires the Rust toolchain (stable channel, 1.75 or
later). Install Rust via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

### Option A: Install via Cargo

```bash
cargo install coven-code --force
```

This downloads, compiles, and installs the binary to `~/.cargo/bin/coven-code`.
That directory is added to `PATH` automatically by `rustup`.

### Option B: Clone and Build

```bash
git clone https://github.com/OpenCoven/coven-code.git
cd coven-code/src-rust

# Debug build (fast to compile, larger binary, extra runtime checks)
cargo build --package coven-code

# Release build (optimised, smaller, suitable for everyday use)
cargo build --release --package coven-code
```

The release binary is placed at:

```
src-rust/target/release/coven-code        # Linux / macOS
src-rust/target\release\coven-code.exe   # Windows
```

Copy it to a directory on your `PATH` as described above.

### Linux system dependencies

On Linux, the build requires ALSA development headers (for the optional voice
feature) and OpenSSL:

```bash
# Debian / Ubuntu
sudo apt-get install -y libasound2-dev libssl-dev pkg-config

# Fedora / RHEL
sudo dnf install -y alsa-lib-devel openssl-devel

# Arch
sudo pacman -S alsa-lib openssl
```

### Optional cargo features

| Feature | Description |
|---------|-------------|
| `voice` | Microphone input / voice prompting |
| `computer-use` | Screenshot capture and mouse/keyboard control |
| `dev_full` | All experimental features combined |

To enable a feature:

```bash
cargo build --release --package coven-code --features voice
cargo build --release --package coven-code --features dev_full
```

### Cross-compiling for Linux aarch64

The release workflow uses [cross](https://github.com/cross-rs/cross) for
aarch64 Linux builds. To reproduce it locally:

```bash
cargo install cross --git https://github.com/cross-rs/cross
cd src-rust
cross build --release --locked --package coven-code --target aarch64-unknown-linux-gnu
```

`cross` manages the Docker sysroot, OpenSSL, and ALSA headers automatically.

---

## Shell Completions

Coven Code does not currently ship a dedicated `completions` subcommand. All
flags can be discovered via `coven-code --help`. If you want basic tab completion
in bash or zsh you can use the generic completion helper built into your shell:

```bash
# bash — add to ~/.bashrc
complete -C coven-code coven-code

# zsh — add to ~/.zshrc (requires compinit)
compdef _gnu_generic coven-code
```

Richer completion scripts may be added in a future release.

---

## Upgrading a source install

```bash
cargo install coven-code --force
```

For binary installs (the recommended path), use `coven-code upgrade` — see
the [Upgrading](#upgrading) section above.

---

## Uninstalling

If you used the install script, remove the install directory:

```bash
rm -rf ~/.coven-code/bin                    # Linux / macOS
# Windows (PowerShell):
Remove-Item -Recurse -Force "$env:USERPROFILE\.coven-code\bin"
```

For manual installs:

```bash
sudo rm /usr/local/bin/coven-code           # if installed system-wide
rm ~/.local/bin/coven-code                  # if installed user-local
```

To also remove all settings and session data:

```bash
rm -rf ~/.coven-code
```

You may also want to remove the `# coven-code` PATH line that the installer
appended to your shell config (`.bashrc`, `.zshrc`, etc.).
