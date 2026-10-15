export const meta = { title: 'Installation' };

export function render() {
  return `
    <h1>Installation</h1>
    <p class="lead">Install the Coven CLI with npm or bun, then run <code>coven</code> or <code>coven tui</code> to open the interactive Coven Code UI.</p>

    <h2>Release Platforms</h2>
    <p>Release archives target Windows x86_64, Linux x86_64/aarch64, and macOS x86_64/aarch64. These build targets do not establish a minimum OS or glibc version; consult the release's native verification results. Windows ARM64 has no native release artifact.</p>

    <h2>Quick Install</h2>

    <pre><code data-lang="bash">npm install -g @opencoven/cli
# or
bun install -g @opencoven/cli</code></pre>

    <pre><code data-lang="bash">coven engine install
coven
coven engine which
coven doctor</code></pre>

    <p>The unified CLI installs the engine version it pins, which may differ from the latest engine release. An explicit <code>COVEN_ENGINE_BIN</code> override wins; otherwise a managed engine takes precedence over a standalone PATH copy. The deprecated <code>@opencoven/coven-code</code> npm package is not published with every release.</p>

    <h2>Standalone Coven Code Binary</h2>

    <p>Download and inspect the installer from the selected <a href="https://github.com/OpenCoven/coven-code/releases" target="_blank" rel="noopener">GitHub Release</a>, then run it locally. The default install directory is <code>~/.coven-code/bin/</code> (Windows: <code>%USERPROFILE%\\.coven-code\\bin</code>). Launch <code>coven-code</code> directly; the installers also create a <code>coven-cave</code> alias.</p>

    <p>The shell installer appends a PATH entry for a recognized shell; PowerShell updates the user PATH unless <code>-NoModifyPath</code> is supplied. The shell installer does not automatically remove macOS quarantine attributes.</p>

    <h3>Installer Flags</h3>

    <table>
      <thead><tr><th>Flag (sh)</th><th>Flag (ps1)</th><th>Effect</th></tr></thead>
      <tbody>
        <tr><td><code>--version &lt;version&gt;</code></td><td><code>-Version &lt;version&gt;</code></td><td>Select a release; omit the leading v</td></tr>
        <tr><td><code>--binary &lt;path&gt;</code></td><td><code>-Binary &lt;path&gt;</code></td><td>Install an already verified local binary</td></tr>
        <tr><td><code>--install-dir &lt;path&gt;</code></td><td><code>-InstallDir &lt;path&gt;</code></td><td>Override install directory</td></tr>
        <tr><td>Not supported</td><td><code>-NoModifyPath</code></td><td>Leave the Windows user PATH unchanged</td></tr>
      </tbody>
    </table>

    <h2>Coven Daemon (Optional)</h2>

    <p>The Coven daemon unlocks the ecosystem features (familiars as agents, daemon-registered skills). It ships separately as the Coven CLI. Use <code>coven doctor</code> to inspect local setup and <code>coven daemon start</code> to start the daemon.</p>

    <pre><code data-lang="bash">npm install -g @opencoven/cli
coven daemon start</code></pre>

    <h2>Upgrading</h2>

    <pre><code data-lang="bash">npm install -g @opencoven/cli@latest
# or
bun install -g @opencoven/cli@latest</code></pre>

    <pre><code data-lang="bash">coven engine install
coven engine which
coven doctor</code></pre>

    <p>For standalone installs, use the chosen release's verified installer or archive. A standalone update does not override the managed engine. Back up affected engine configuration and sessions and retain a verified prior binary before upgrading. Standalone data normally lives in <code>~/.coven-code</code>; under Coven, in <code>~/.coven/code</code>. Inspect environment overrides and migration symlinks before backup or removal.</p>

    <p>For releases with <code>SHA256SUMS</code>, verify the selected archive against the exact entry from the same release before extraction. The next-release checksum-verifying installers require the full five-archive manifest and reject historical releases without it, including v0.8.0. Do not bypass verification for rollback; use an independently verified prior artifact and compatible data. Same-release HTTPS checksums are not independently signed attestations.</p>

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
cargo build --locked --package claurst

# Release build (optimised, smaller, for everyday use)
cargo build --release --locked --package claurst</code></pre>

    <h3>Build Requirements</h3>

    <p>Use the stable Rust toolchain used by CI and the checked-in lockfile. On Debian/Ubuntu the release workflow installs <code>libasound2-dev</code> and <code>pkg-config</code>. Voice is enabled by default. The CLI's opt-in feature is <code>steer</code>; do not pass unrelated internal-crate features to the CLI package.</p>

    <h2>CLI Help</h2>
    <p>Use <code>coven-code --help</code> for supported flags. Source builds provide the <code>coven-code</code> binary; release installers create the alias separately.</p>

    <h2>Uninstalling</h2>

    <p>Remove the executable and installer-created alias from the actual installation directory and remove only its PATH entry. For a legacy npm installation use <code>npm uninstall -g @opencoven/coven-code</code>; for a Cargo install use <code>cargo uninstall claurst</code>.</p>

    <p>Keep settings and sessions unless you intend to remove them. Do not delete all of <code>~/.coven</code>: it contains shared ecosystem state. Identify and back up the effective engine home, inspect migration symlinks, and remove only the engine-owned data you selected.</p>

    <p>See <a href="https://github.com/OpenCoven/coven-code/blob/main/docs/installation.md" target="_blank" rel="noopener">the full installation reference</a> for cross-compiling to Linux aarch64, optional cargo features, and user-local installs without sudo.</p>
  `;
}
