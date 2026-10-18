$ErrorActionPreference = 'Stop'
$env:COVEN_CODE_INSTALLER_TEST = '1'
. $env:ROLLBACK_INSTALLER

$InstallDir = $env:ROLLBACK_TARGET
$source = $env:ROLLBACK_SOURCE
$target = Join-Path $InstallDir 'coven-code.exe'
$alias = Join-Path $InstallDir 'coven-cave.exe'
$short = Join-Path $InstallDir 'coven.exe'

function Copy-Item {
    param(
        [Parameter(Position=0)][string]$Path,
        [Parameter(Position=1)][string]$Destination,
        [string]$LiteralPath,
        [switch]$Force,
        [string]$ErrorAction
    )
    if ($Destination -eq $alias) { throw 'fixture alias copy failure' }
    Microsoft.PowerShell.Management\Copy-Item @PSBoundParameters
}

function Move-Item {
    param(
        [Parameter(Position=0)][string]$Path,
        [Parameter(Position=1)][string]$Destination,
        [string]$LiteralPath,
        [switch]$Force,
        [string]$ErrorAction
    )
    if ($Destination -eq $alias -and $LiteralPath -like '*new-coven-cave.exe') {
        throw 'fixture alias move failure'
    }
    Microsoft.PowerShell.Management\Move-Item @PSBoundParameters
}

$failed = $false
try {
    if ($env:ROLLBACK_MODE -eq 'download') {
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
        $install = { param([string]$Source) Install-Binary $Source }
        $boundaries = [pscustomobject]@{ Download = $download; Expand = $expand; Install = $install }
        Download-And-Install '9.9.9' 'x86_64' $boundaries
    } else {
        Install-Binary $source
    }
} catch { $failed = $true }
if (-not $failed) { throw 'Alias publication unexpectedly succeeded' }

if ([IO.File]::ReadAllText($target) -ne 'old engine' -or
    [IO.File]::ReadAllText($alias) -ne 'old alias' -or
    [IO.File]::ReadAllText($short) -ne 'old short') {
    throw 'Failed installation did not restore the previous executables'
}
if (@(Get-ChildItem $InstallDir -Name).Count -ne 3) {
    throw 'Failed installation left staging or backup files'
}
