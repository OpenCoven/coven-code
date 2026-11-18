# Drives Download-And-Install with an existing engine that differs in bytes
# from the downloaded one (#369). CONSENT_REPORTED_VERSION is the version the
# existing engine reports; CONSENT_EXPECT is 'installed' or 'kept'.
$ErrorActionPreference = 'Stop'
$env:COVEN_CODE_INSTALLER_TEST = '1'
. $env:CONSENT_INSTALLER

$InstallDir = $env:CONSENT_TARGET
$target = Join-Path $InstallDir 'coven-code.exe'
$alias = Join-Path $InstallDir 'coven-cave.exe'
[IO.File]::WriteAllText($target, 'old engine')
[IO.File]::WriteAllText($alias, 'old alias')

# The fixture engines are not executables, so stand in for asking one.
function Get-EngineVersion([string]$Path) {
    if ($Path -eq $target) { return $env:CONSENT_REPORTED_VERSION }
    return 'unknown'
}

$script:archiveBytes = [Text.Encoding]::UTF8.GetBytes('inert archive')
$sha = [Security.Cryptography.SHA256]::Create()
try { $digest = ([BitConverter]::ToString($sha.ComputeHash($script:archiveBytes))).Replace('-', '').ToLowerInvariant() }
finally { $sha.Dispose() }
$names = @(
    'coven-code-linux-aarch64.tar.gz', 'coven-code-linux-x86_64.tar.gz',
    'coven-code-macos-aarch64.tar.gz', 'coven-code-macos-x86_64.tar.gz',
    'coven-code-windows-x86_64.zip'
)
$script:manifest = (($names | ForEach-Object {
    $hash = if ($_ -eq 'coven-code-windows-x86_64.zip') { $digest } else { '0' * 64 }
    "$hash  $_"
}) -join "`n") + "`n"
$download = {
    param([string]$Uri, [string]$OutFile)
    if ($Uri.EndsWith('/SHA256SUMS')) {
        [IO.File]::WriteAllText($OutFile, $script:manifest)
    } else {
        [IO.File]::WriteAllBytes($OutFile, $script:archiveBytes)
    }
}
$expand = {
    param([string]$ArchivePath, [string]$DestinationPath)
    [IO.File]::WriteAllText((Join-Path $DestinationPath 'coven-code.exe'), 'new engine')
}
$install = { param([string]$Source, [string]$Version) Install-Binary $Source $Version }
$boundaries = [pscustomobject]@{ Download = $download; Expand = $expand; Install = $install }

$failure = $null
try { Download-And-Install '9.9.9' 'x86_64' $boundaries } catch { $failure = $_ }

if ($env:CONSENT_EXPECT -eq 'installed') {
    if ($null -ne $failure) { throw "Install failed: $failure" }
    if ([IO.File]::ReadAllText($target) -ne 'new engine' -or
        [IO.File]::ReadAllText($alias) -ne 'new engine') {
        throw 'The engine was not replaced'
    }
} else {
    if ($null -eq $failure) { throw 'A different engine was replaced without consent' }
    if ([IO.File]::ReadAllText($target) -ne 'old engine' -or
        [IO.File]::ReadAllText($alias) -ne 'old alias') {
        throw 'A refused install changed the existing engine'
    }
    if (@(Get-ChildItem $InstallDir -Name).Count -ne 2) {
        throw 'A refused install left staging or backup files'
    }
}
