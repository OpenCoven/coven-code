# Coven Code installer for Windows (PowerShell).
#
# Usage (one-liner):
#   irm https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 | iex
#
# Or download and run locally:
#   Invoke-WebRequest https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 -OutFile install.ps1
#   .\install.ps1

[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$Binary = "",
    [string]$InstallDir = "",
    [switch]$NoModifyPath,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'

$App = 'coven-code'
$Repo = 'OpenCoven/coven-code'

function Write-Info($msg)    { Write-Host $msg }
function Write-Success($msg) { Write-Host $msg -ForegroundColor Green }
function Write-Warn($msg)    { Write-Host $msg -ForegroundColor Yellow }
function Write-Err($msg)     { Write-Host $msg -ForegroundColor Red }
function Write-Muted($msg)   { Write-Host $msg -ForegroundColor DarkGray }

function Get-ExpectedChecksum([string]$ManifestPath, [string]$ArchiveName) {
    $expectedNames = @(
        'coven-code-linux-aarch64.tar.gz',
        'coven-code-linux-x86_64.tar.gz',
        'coven-code-macos-aarch64.tar.gz',
        'coven-code-macos-x86_64.tar.gz',
        'coven-code-windows-x86_64.zip'
    )
    $seen = [System.Collections.Generic.Dictionary[string, string]]::new([System.StringComparer]::Ordinal)
    foreach ($line in [IO.File]::ReadAllLines($ManifestPath)) {
        if ($line -notmatch '^([0-9A-Fa-f]{64})  ([A-Za-z0-9._-]+)$') {
            throw 'SHA256SUMS contains an unsupported or malformed line'
        }
        $name = $Matches[2]
        if ($expectedNames -cnotcontains $name -or $seen.ContainsKey($name)) {
            throw 'SHA256SUMS contains an unknown or duplicate filename'
        }
        $seen[$name] = $Matches[1].ToLowerInvariant()
    }
    if ($seen.Count -ne $expectedNames.Count) {
        throw 'SHA256SUMS is incomplete'
    }
    foreach ($name in $expectedNames) {
        if (-not $seen.ContainsKey($name)) { throw 'SHA256SUMS is incomplete' }
    }
    if (-not $seen.ContainsKey($ArchiveName)) { throw 'SHA256SUMS does not contain the selected archive' }
    return $seen[$ArchiveName]
}

# Windows PowerShell 5.1 does not always load this assembly from type literals.
Add-Type -AssemblyName System.Net.Http

function Invoke-HttpsDownload([string]$Uri, [string]$OutFile, [scriptblock]$Request = $null, [TimeSpan]$Timeout = [TimeSpan]::FromMinutes(5), [System.Net.Http.HttpClient]$HttpClient = $null) {
    $current = [Uri]$Uri
    $handler = $null
    $client = $HttpClient
    $ownsClient = $null -eq $client
    $deadline = [System.Threading.CancellationTokenSource]::new($Timeout)
    if ($null -eq $Request -and $ownsClient) {
        $handler = [System.Net.Http.HttpClientHandler]::new()
        $handler.AllowAutoRedirect = $false
        $client = [System.Net.Http.HttpClient]::new($handler)
        $client.Timeout = [System.Threading.Timeout]::InfiniteTimeSpan
    }
    try {
        for ($redirects = 0; $redirects -le 5; $redirects++) {
            $deadline.Token.ThrowIfCancellationRequested()
            if (-not $current.IsAbsoluteUri -or $current.Scheme -ne 'https') {
                throw "Refusing non-HTTPS download URI: $current"
            }
            if ($null -ne $Request) {
                $result = & $Request $current $deadline.Token
                $deadline.Token.ThrowIfCancellationRequested()
                $status = [int]$result.StatusCode
                if ($status -in @(301, 302, 303, 307, 308)) {
                    if ($redirects -eq 5 -or [string]::IsNullOrEmpty($result.Location)) {
                        throw 'Download redirect limit exceeded or Location header missing'
                    }
                    $current = [Uri]::new($current, [string]$result.Location)
                    continue
                }
                if ($status -ne 200) { throw "HTTP $status downloading $current" }
                [IO.File]::WriteAllBytes($OutFile, [byte[]]$result.Bytes)
                return
            }
            $response = $client.GetAsync($current, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead, $deadline.Token).GetAwaiter().GetResult()
            try {
                $status = [int]$response.StatusCode
                if ($status -in @(301, 302, 303, 307, 308)) {
                    if ($redirects -eq 5 -or $null -eq $response.Headers.Location) {
                        throw 'Download redirect limit exceeded or Location header missing'
                    }
                    $current = [Uri]::new($current, $response.Headers.Location)
                    continue
                }
                if ($status -ne 200) { throw "HTTP $status downloading $current" }
                $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
                $outputStream = [IO.File]::Create($OutFile)
                try { $inputStream.CopyToAsync($outputStream, 81920, $deadline.Token).GetAwaiter().GetResult() } finally {
                    $outputStream.Dispose()
                    $inputStream.Dispose()
                }
                return
            } finally { $response.Dispose() }
        }
    } catch {
        if ($deadline.IsCancellationRequested) { throw 'Download timed out' }
        throw
    } finally {
        $deadline.Dispose()
        if ($ownsClient -and $null -ne $client) { $client.Dispose() }
        if ($null -ne $handler) { $handler.Dispose() }
    }
    throw 'Download redirect limit exceeded'
}

function Assert-ArchiveChecksum([string]$ManifestPath, [string]$ArchiveName, [string]$ArchivePath) {
    $expected = Get-ExpectedChecksum $ManifestPath $ArchiveName
    $actual = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "Checksum mismatch for $ArchiveName" }
}

function Show-Usage {
@"
Coven Code installer (Windows)

Usage: install.ps1 [options]

Options:
    -Help                   Show this help
    -Version <version>      Install a specific version (e.g., 0.1.0)
    -Binary <path>          Install from a local binary instead of downloading
    -InstallDir <path>      Override install location (default: %USERPROFILE%\.coven-code\bin)
    -NoModifyPath           Don't add the install dir to user PATH

Examples:
    irm https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 | iex
    .\install.ps1 -Version 0.1.0
    .\install.ps1 -Binary C:\path\to\coven-code.exe
"@
}

if ($Help) { Show-Usage; exit 0 }

# ----- Detect architecture -----
function Get-Arch {
    $procArch = $env:PROCESSOR_ARCHITECTURE
    if ($null -eq $procArch) { $procArch = '' }
    switch ($procArch.ToLower()) {
        'amd64'   { return 'x86_64' }
        'x86'     { return 'x86_64' }   # rare; ship 64-bit anyway
        'arm64'   {
            Write-Warn "ARM64 Windows is not currently supported in releases. Falling back to x86_64."
            return 'x86_64'
        }
        default   {
            Write-Warn "Unknown architecture '$procArch'. Defaulting to x86_64."
            return 'x86_64'
        }
    }
}

# ----- Resolve version (latest if not provided) -----
function Resolve-Version {
    if (-not [string]::IsNullOrEmpty($script:Version)) {
        return ($script:Version -replace '^v', '')
    }
    try {
        $resp = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'coven-code-installer' }
        $tag = $resp.tag_name
        if ([string]::IsNullOrEmpty($tag)) { throw "no tag_name in response" }
        return ($tag -replace '^v', '')
    } catch {
        Write-Err "Failed to fetch latest version from GitHub API: $_"
        exit 1
    }
}

# ----- Already-installed check -----
function Check-Existing($desiredVersion) {
    $existing = Get-Command coven-code -ErrorAction SilentlyContinue
    if ($null -eq $existing) { return }
    try {
        $vline = (& coven-code --version) 2>&1 | Select-Object -First 1
        $installed = ($vline -split '\s+')[-1]
    } catch {
        $installed = 'unknown'
    }
    if ($installed -eq $desiredVersion) {
        Write-Muted "Version $desiredVersion already installed at $($existing.Source)"
        Write-Muted "Use -Version to install a different one."
        exit 0
    }
    Write-Muted "Found existing coven-code at $($existing.Source) (v$installed) - upgrading to v$desiredVersion"
}

# ----- Download & extract -----
function Download-And-Install($desiredVersion, $arch, $Boundaries = $null) {
    $archive = "coven-code-windows-$arch.zip"
    $url = "https://github.com/$Repo/releases/download/v$desiredVersion/$archive"
    $checksumUrl = "https://github.com/$Repo/releases/download/v$desiredVersion/SHA256SUMS"
    $tmpRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("coven-code-install-" + [System.Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmpRoot -Force | Out-Null

    $zipPath = Join-Path $tmpRoot $archive
    $checksumPath = Join-Path $tmpRoot 'SHA256SUMS'
    $extractDir = Join-Path $tmpRoot "extract"
    New-Item -ItemType Directory -Path $extractDir -Force | Out-Null

    Write-Info "Installing coven-code v$desiredVersion (windows-$arch)"
    Write-Muted "Downloading $url"
    try {
        if ($null -eq $Boundaries) {
            Invoke-HttpsDownload $checksumUrl $checksumPath
            Invoke-HttpsDownload $url $zipPath
        } else {
            & $Boundaries.Download $checksumUrl $checksumPath
            & $Boundaries.Download $url $zipPath
        }
        Assert-ArchiveChecksum $checksumPath $archive $zipPath
        Write-Muted "Verified SHA-256 for $archive"
    } catch {
        Write-Err "Download failed: $_"
        Write-Info ("Check that release v$desiredVersion exists for windows-" + $arch + ":")
        Write-Info "  https://github.com/$Repo/releases/tag/v$desiredVersion"
        Remove-Item -Recurse -Force $tmpRoot -ErrorAction SilentlyContinue
        if ($null -ne $Boundaries) { throw }
        exit 1
    }

    Write-Muted "Extracting..."
    try {
        if ($null -eq $Boundaries) {
            Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force
        } else {
            & $Boundaries.Expand $zipPath $extractDir
        }
    } catch {
        Write-Err "Extract failed: $_"
        Remove-Item -Recurse -Force $tmpRoot -ErrorAction SilentlyContinue
        if ($null -ne $Boundaries) { throw }
        exit 1
    }

    $extractedExe = Join-Path $extractDir 'coven-code.exe'
    if (-not (Test-Path $extractedExe)) {
        Write-Err "Archive did not contain expected binary 'coven-code.exe'"
        Get-ChildItem -Recurse $extractDir | Format-Table FullName
        Remove-Item -Recurse -Force $tmpRoot -ErrorAction SilentlyContinue
        exit 1
    }

    if ($null -eq $Boundaries) {
        Install-Binary $extractedExe
    } else {
        & $Boundaries.Install $extractedExe
    }
    Remove-Item -Recurse -Force $tmpRoot -ErrorAction SilentlyContinue
}

function Install-FromBinary {
    if (-not (Test-Path $script:Binary)) {
        Write-Err "Binary not found at $script:Binary"
        exit 1
    }
    Write-Info "Installing coven-code from $script:Binary"
    # Stage before Install-Binary renames existing executables: the source may
    # itself be the installed engine or alias.
    $stagedBinary = [System.IO.Path]::GetTempFileName()
    try {
        Copy-Item -LiteralPath $script:Binary -Destination $stagedBinary -Force
        Install-Binary $stagedBinary
    } finally {
        Remove-Item -LiteralPath $stagedBinary -Force -ErrorAction SilentlyContinue
    }
}

function Install-Binary($source) {
    $target = Join-Path $InstallDir 'coven-code.exe'
    $aliasTarget = Join-Path $InstallDir 'coven-cave.exe'
    # `coven.exe` belongs to the Coven daemon CLI (@opencoven/cli); older
    # installers shipped it as an alias, so clean it up on upgrade.
    $staleShort = Join-Path $InstallDir 'coven.exe'

    # The currently running coven-code.exe (if any) holds an exclusive file lock on
    # Windows.  Try to swap by renaming the old one first.
    foreach ($path in @($target, $aliasTarget, $staleShort)) {
        if (-not (Test-Path $path)) { continue }
        $stale = "$path.old"
        if (Test-Path $stale) { Remove-Item -Force $stale -ErrorAction SilentlyContinue }
        try { Move-Item -Force $path $stale } catch { }
    }

    Copy-Item -Force $source $target
    Copy-Item -Force $source $aliasTarget
    Write-Success "Installed: $target"
    Write-Success "Installed alias: $aliasTarget"
}

# ----- PATH modification -----
function Add-ToUserPath {
    if ($NoModifyPath) { return }

    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($null -eq $current) { $current = '' }

    # Already on PATH?
    $paths = $current -split ';' | Where-Object { $_ -ne '' }
    foreach ($p in $paths) {
        if ($p.TrimEnd('\') -ieq $InstallDir.TrimEnd('\')) {
            Write-Muted "Install dir already on user PATH: $InstallDir"
            return
        }
    }

    if ([string]::IsNullOrEmpty($current)) {
        $newPath = $InstallDir
    } else {
        $newPath = $InstallDir + ';' + $current
    }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')

    # Make it visible in this session too so coven-code --version works immediately.
    $env:Path = $InstallDir + ';' + $env:Path

    Write-Success ("Added " + $InstallDir + " to user PATH")
    Write-Muted "Open a new terminal for the change to take effect everywhere."
}

# ----- GitHub Actions hint -----
function GithubPathHint {
    if ($env:GITHUB_ACTIONS -eq 'true' -and -not [string]::IsNullOrEmpty($env:GITHUB_PATH)) {
        Add-Content -Path $env:GITHUB_PATH -Value $InstallDir
        Write-Info "Added $InstallDir to `$GITHUB_PATH"
    }
}

# ----- Main flow -----
if ($env:COVEN_CODE_INSTALLER_TEST -eq '1') { return }

# ----- Resolve install directory -----
if ([string]::IsNullOrEmpty($InstallDir)) {
    $InstallDir = Join-Path $env:USERPROFILE ".coven-code\bin"
}
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

Write-Info "Tip: the unified Coven CLI installs and manages this engine for you -- 'npm install -g @opencoven/cli', then run 'coven'. This standalone installer still works."

if (-not [string]::IsNullOrEmpty($Binary)) {
    Install-FromBinary
} else {
    $arch = Get-Arch
    $desiredVersion = Resolve-Version
    Check-Existing $desiredVersion
    Download-And-Install $desiredVersion $arch
}

Add-ToUserPath
GithubPathHint

Write-Host ""
Write-Success "coven-code is installed!"

# The unified Coven CLI keeps its own engine under %USERPROFILE%\.coven\engine
# and runs that one ahead of anything on PATH. Say so, or this copy looks like
# an upgrade that `coven` ignores.
$managedRoot = Join-Path $env:USERPROFILE '.coven\engine'
$managedCurrent = Join-Path $managedRoot 'current'
if (Test-Path $managedCurrent -PathType Leaf) {
    $managedVersion = (Get-Content $managedCurrent -Raw -ErrorAction SilentlyContinue)
    if (-not [string]::IsNullOrWhiteSpace($managedVersion)) {
        $managedVersion = $managedVersion.Trim()
        $managedBinary = Join-Path (Join-Path $managedRoot $managedVersion) 'coven-code.exe'
        if (Test-Path $managedBinary -PathType Leaf) {
            Write-Warn "The Coven CLI already manages an engine at $managedRoot\$managedVersion and runs that one, not this copy."
            Write-Warn "Upgrade the engine 'coven' runs with: coven engine install. Keep this copy only if you launch coven-code directly; 'coven doctor' reports which one is in use."
        }
    }
}
Write-Host ""
Write-Muted  "Quickstart:"
Write-Muted  "  # Set an API key"
Write-Host   "  `$env:ANTHROPIC_API_KEY = 'sk-ant-...'"
Write-Host   ""
Write-Muted  "  # Open a new terminal, then:"
Write-Success "  coven-code             "
Write-Success "  coven-cave             "
Write-Muted  "  # or"
Write-Success "  coven-code -p `"...`"      "
Write-Host   ""
Write-Muted  "Docs: https://github.com/$Repo"
