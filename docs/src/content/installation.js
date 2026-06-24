export const meta = { title: 'Installation' };

export function render() {
  return `
    <h1>Installation</h1>
    <p class="lead">Install the Coven CLI with npm or bun, then run <code>coven</code> or <code>coven tui</code> to open the interactive Coven Code UI.</p>

    <h2>System Requirements</h2>

    <table>
      <thead><tr><th>Platform</th><th>Architecture</th><th>Minimum OS</th></tr></thead>
      <tbody>
        <tr><td>Windows</td><td>x86_64</td><td>Windows 10 / Server 2019</td></tr>
        <tr><td>Linux</td><td>x86_64</td><td>glibc 2.17+ (2014+ distros)</td></tr>
        <tr><td>Linux</td><td>aarch64</td><td>glibc 2.17+ (Raspberry Pi 4, AWS Graviton)</td></tr>
        <tr><td>macOS</td><td>x86_64</td><td>macOS 11 Big Sur</td></tr>
        <tr><td>macOS</td><td>aarch64</td><td>macOS 11 Big Sur (M1/M2/M3)</td></tr>
      </tbody>
    </table>

    <h2>Quick Install</h2>

    <pre><code data-lang="bash">npm install -g @opencoven/coven
# or
bun install -g @opencoven/coven</code></pre>

    <p>The installed command is <code>coven</code>. Run <code>coven</code> with no arguments, or <code>coven tui</code> explicitly, for the interactive UI. Use <code>coven doctor</code> to inspect local setup, <code>coven daemon start</code> to start the local daemon, and <code>coven run &lt;harness&gt; "&lt;task&gt;"</code> for direct harness sessions.</p>

    <pre><code data-lang="bash">npx @opencoven/coven
bunx @opencoven/coven</code></pre>

    <h2>Standalone Coven Code Binary</h2>

    <h3>Linux / macOS</h3>

    <pre><code data-lang="bash">curl -fsSL https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh | bash</code></pre>

    <h3>Windows (PowerShell)</h3>

    <pre><code data-lang="bash">irm https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 | iex</code></pre>

    <p>Both installers detect platform/arch, download the matching archive, drop <code>coven-code</code> into <code>~/.coven-code/bin/</code>, and add that directory to your <code>PATH</code>. On macOS, they also strip the Gatekeeper quarantine attribute so the unsigned binary runs without a manual override.</p>

    <h3>Installer Flags</h3>

    <table>
      <thead><tr><th>Flag (sh)</th><th>Flag (ps1)</th><th>Effect</th></tr></thead>
      <tbody>
        <tr><td><code>--version 0.1.0</code></td><td><code>-Version 0.1.0</code></td><td>Install a specific version</td></tr>
        <tr><td><code>--binary &lt;path&gt;</code></td><td><code>-Binary &lt;path&gt;</code></td><td>Install from a local file (skip download)</td></tr>
        <tr><td><code>--install-dir &lt;path&gt;</code></td><td><code>-InstallDir &lt;path&gt;</code></td><td>Override install directory</td></tr>
        <tr><td><code>--no-modify-path</code></td><td><code>-NoModifyPath</code></td><td>Don't touch shell config / PATH</td></tr>
      </tbody>
    </table>

    <h2>Coven Code npm Package</h2>

    <p>Prefer <code>@opencoven/coven</code> for the user-facing <code>coven</code> CLI. The lower-level Coven Code package installs the <code>coven-code</code> binary directly.</p>

    <pre><code data-lang="bash">npm install -g @opencoven/coven-code
# or
bun install -g @opencoven/coven-code</code></pre>

    <p>The postinstall script downloads the correct pre-built binary from GitHub Releases — no compilation needed. Or run without a permanent install:</p>

    <pre><code data-lang="bash">npx @opencoven/coven-code
bunx @opencoven/coven-code</code></pre>

    <h2>Upgrading</h2>

    <pre><code data-lang="bash">npm install -g @opencoven/coven@latest
# or
bun install -g @opencoven/coven@latest</code></pre>

    <p>Settings under <code>~/.coven/</code> and <code>~/.coven-code/</code> are preserved.</p>

    <h2>Manual Install</h2>

    <p>Grab archives from <a href="https://github.com/OpenCoven/coven-code/releases" target="_blank" rel="noopener">GitHub Releases</a>:</p>

    <table>
      <thead><tr><th>Archive</th><th>Platform</th></tr></thead>
      <tbody>
        <tr><td><code>coven-code-windows-x86_64.zip</code></td><td>Windows 64-bit</td></tr>
        <tr><td><code>coven-code-linux-x86_64.tar.gz</code></td><td>Linux x86_64</td></tr>
        <tr><td><code>coven-code-linux-aarch64.tar.gz</code></td><td>Linux ARM64</td></tr>
        <tr><td><code>coven-code-macos-x86_64.tar.gz</code></td><td>macOS Intel</td></tr>
        <tr><td><code>coven-code-macos-aarch64.tar.gz</code></td><td>macOS Apple Silicon</td></tr>
      </tbody>
    </table>

    <h2>From Source</h2>

    <h3>From a Clone</h3>

    <pre><code data-lang="bash">git clone https://github.com/OpenCoven/coven-code
cd coven-code/src-rust
cargo install --path crates/cli --locked</code></pre>

    <h3>Clone and Build</h3>

    <pre><code data-lang="bash">git clone https://github.com/OpenCoven/coven-code
cd coven-code/src-rust

# Debug build (fast to compile, larger binary)
cargo build --package claurst

# Release build (optimised, smaller, for everyday use)
cargo build --release --package claurst</code></pre>

    <h3>Linux System Dependencies</h3>

    <pre><code data-lang="bash"># Debian / Ubuntu
sudo apt install build-essential pkg-config libssl-dev

# Fedora / RHEL
sudo dnf install gcc pkgconfig openssl-devel

# Arch
sudo pacman -S base-devel pkgconf openssl</code></pre>

    <h2>Shell Completions</h2>

    <p>Coven does not currently ship a dedicated completions subcommand. All flags can be discovered via <code>coven --help</code>. If you want basic tab completion in bash or zsh, use the generic completion helper built into your shell:</p>

    <pre><code data-lang="bash"># bash — add to ~/.bashrc
complete -C coven coven

# zsh — add to ~/.zshrc (requires compinit)
compdef _gnu_generic coven</code></pre>

    <h2>Uninstalling</h2>

    <pre><code data-lang="bash">npm uninstall -g @opencoven/coven
# or
bun remove -g @opencoven/coven

rm -rf ~/.coven ~/.coven-code     # Linux / macOS

# Windows (PowerShell):
Remove-Item -Recurse -Force $env:USERPROFILE\\.coven, $env:USERPROFILE\\.coven-code</code></pre>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/installation.md" target="_blank" rel="noopener">the full installation reference</a> for cross-compiling to Linux aarch64, optional cargo features, and user-local installs without sudo.</p>
  `;
}
