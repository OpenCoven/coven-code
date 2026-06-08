export const meta = { title: 'Installation' };

export function render() {
  return `
    <h1>Installation</h1>
    <p class="lead">A statically-linked Rust binary with no runtime dependencies. Install via the official installer script, npm, or build from source.</p>

    <h2>System requirements</h2>

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

    <h2>Quick install</h2>

    <h3>Linux / macOS</h3>

    <pre><code data-lang="bash">curl -fsSL https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh | bash</code></pre>

    <h3>Windows (PowerShell)</h3>

    <pre><code data-lang="bash">irm https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 | iex</code></pre>

    <p>Both installers detect platform/arch, download the matching archive, drop <code>coven-code</code> into <code>~/.coven-code/bin/</code>, and add that directory to your <code>PATH</code>. On macOS, they also strip the Gatekeeper quarantine attribute so the unsigned binary runs without a manual override.</p>

    <h3>Installer flags</h3>

    <table>
      <thead><tr><th>Flag (sh)</th><th>Flag (ps1)</th><th>Effect</th></tr></thead>
      <tbody>
        <tr><td><code>--version 0.1.0</code></td><td><code>-Version 0.1.0</code></td><td>Install a specific version</td></tr>
        <tr><td><code>--binary &lt;path&gt;</code></td><td><code>-Binary &lt;path&gt;</code></td><td>Install from a local file (skip download)</td></tr>
        <tr><td><code>--install-dir &lt;path&gt;</code></td><td><code>-InstallDir &lt;path&gt;</code></td><td>Override install directory</td></tr>
        <tr><td><code>--no-modify-path</code></td><td><code>-NoModifyPath</code></td><td>Don't touch shell config / PATH</td></tr>
      </tbody>
    </table>

    <h2>Via npm / bun</h2>

    <pre><code data-lang="bash">npm install -g coven-code
# or
bun install -g coven-code</code></pre>

    <p>The postinstall script downloads the correct pre-built binary from GitHub Releases — no compilation needed. Or run without a permanent install:</p>

    <pre><code data-lang="bash">npx coven-code
bunx coven-code</code></pre>

    <h2>Upgrading</h2>

    <pre><code data-lang="bash">coven-code upgrade                  # to the latest release
coven-code upgrade --version 0.1.0  # pin to a specific version
coven-code upgrade --force          # reinstall the same version</code></pre>

    <p>Settings under <code>~/.coven-code/</code> are preserved.</p>

    <h2>Manual install</h2>

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

    <h2>From source</h2>

    <h3>Via Cargo</h3>

    <pre><code data-lang="bash">cargo install --git https://github.com/OpenCoven/coven-code coven-code-cli</code></pre>

    <h3>Clone and build</h3>

    <pre><code data-lang="bash">git clone https://github.com/OpenCoven/coven-code
cd coven-code/src-rust

# Debug build (fast to compile, larger binary)
cargo build

# Release build (optimised, smaller, for everyday use)
cargo build --release</code></pre>

    <h3>Linux system dependencies</h3>

    <pre><code data-lang="bash"># Debian / Ubuntu
sudo apt install build-essential pkg-config libssl-dev

# Fedora / RHEL
sudo dnf install gcc pkgconfig openssl-devel

# Arch
sudo pacman -S base-devel pkgconf openssl</code></pre>

    <h2>Shell completions</h2>

    <pre><code data-lang="bash"># bash — add to ~/.bashrc
eval "$(coven-code completion bash)"

# zsh — add to ~/.zshrc (requires compinit)
eval "$(coven-code completion zsh)"</code></pre>

    <h2>Uninstalling</h2>

    <pre><code data-lang="bash">rm -rf ~/.coven-code              # Linux / macOS

# Windows (PowerShell):
Remove-Item -Recurse -Force $env:USERPROFILE\\.coven-code</code></pre>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/installation.md" target="_blank" rel="noopener">the full installation reference</a> for cross-compiling to Linux aarch64, optional cargo features, and user-local installs without sudo.</p>
  `;
}
