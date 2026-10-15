$ErrorActionPreference = 'Stop'
$RepoRoot = (Get-Location).Path
$env:COVEN_CODE_INSTALLER_TEST = '1'
. (Join-Path $RepoRoot 'install.ps1')

$names = @(
    'coven-code-linux-aarch64.tar.gz',
    'coven-code-linux-x86_64.tar.gz',
    'coven-code-macos-aarch64.tar.gz',
    'coven-code-macos-x86_64.tar.gz',
    'coven-code-windows-x86_64.zip'
)
$selected = 'coven-code-windows-x86_64.zip'
$digest = ('ab' * 32)

function Manifest([string]$selectedDigest = $digest) {
    return (($names | ForEach-Object {
        $hash = if ($_ -eq $selected) { $selectedDigest } else { '0' * 64 }
        "$hash  $_"
    }) -join "`n") + "`n"
}

function Assert-Fails([string]$label, [string]$body) {
    $path = Join-Path ([System.IO.Path]::GetTempPath()) ("coven-manifest-" + [guid]::NewGuid().ToString('N'))
    try {
        [IO.File]::WriteAllText($path, $body)
        $failed = $false
        try { Get-ExpectedChecksum $path $selected | Out-Null } catch { $failed = $true }
        if (-not $failed) { throw "$label manifest unexpectedly accepted" }
    } finally { Remove-Item $path -Force -ErrorAction SilentlyContinue }
}

$validPath = Join-Path ([System.IO.Path]::GetTempPath()) ("coven-manifest-" + [guid]::NewGuid().ToString('N'))
try {
    [IO.File]::WriteAllText($validPath, (Manifest))
    if ((Get-ExpectedChecksum $validPath $selected) -ne $digest) { throw 'lowercase digest rejected' }
    [IO.File]::WriteAllText($validPath, (Manifest $digest.ToUpperInvariant()))
    if ((Get-ExpectedChecksum $validPath $selected) -ne $digest) { throw 'uppercase digest rejected' }
} finally { Remove-Item $validPath -Force -ErrorAction SilentlyContinue }

Assert-Fails 'malformed' 'not-a-manifest'
Assert-Fails 'missing' ((Manifest) -replace '(?m)^.{64}  coven-code-macos-aarch64.tar.gz\r?\n', '')
Assert-Fails 'duplicate' ((Manifest) + "$digest  $selected`n")
Assert-Fails 'wrong filename' ((Manifest) -replace [regex]::Escape($selected), '../evil.zip')
Assert-Fails 'uppercase filename' ((Manifest) -replace [regex]::Escape($selected), $selected.ToUpperInvariant())
Assert-Fails 'bad digest' ((Manifest) -replace "(?m)^$digest  $([regex]::Escape($selected))$", "$('z' * 64)  $selected")

$archivePath = Join-Path ([System.IO.Path]::GetTempPath()) ("coven-archive-" + [guid]::NewGuid().ToString('N'))
$manifestPath = Join-Path ([System.IO.Path]::GetTempPath()) ("coven-manifest-" + [guid]::NewGuid().ToString('N'))
try {
    [IO.File]::WriteAllText($archivePath, 'inert archive bytes')
    [IO.File]::WriteAllText($manifestPath, (Manifest))
    $failed = $false
    try { Assert-ArchiveChecksum $manifestPath $selected $archivePath } catch { $failed = $true }
    if (-not $failed) { throw 'digest mismatch unexpectedly accepted' }
} finally {
    Remove-Item $archivePath, $manifestPath -Force -ErrorAction SilentlyContinue
}

$source = [IO.File]::ReadAllText((Join-Path $RepoRoot 'install.ps1'))
if ($source -notmatch '\.AllowAutoRedirect\s*=\s*\$false' -or
    $source -notmatch "\.Scheme\s+-ne\s+'https'" -or
    $source -notmatch '\$redirects\s+-le\s+5') {
    throw 'HTTPS-only bounded manual redirect contract is missing'
}

$toolArchive = Join-Path ([System.IO.Path]::GetTempPath()) ("coven-archive-" + [guid]::NewGuid().ToString('N'))
$toolManifest = Join-Path ([System.IO.Path]::GetTempPath()) ("coven-manifest-" + [guid]::NewGuid().ToString('N'))
[IO.File]::WriteAllText($toolArchive, 'inert')
[IO.File]::WriteAllText($toolManifest, (Manifest))
function Get-FileHash { throw 'hash tool unavailable' }
$failed = $false
try { Assert-ArchiveChecksum $toolManifest $selected $toolArchive } catch { $failed = $true }
if (-not $failed) { throw 'missing hash tool unexpectedly accepted' }
Remove-Item Function:Get-FileHash
Remove-Item $toolArchive, $toolManifest -Force -ErrorAction SilentlyContinue

function Invoke-IntegrationCase([string]$Scenario, [bool]$ShouldPass) {
    $caseRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("coven-ps-flow-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $caseRoot | Out-Null
    $existing = Join-Path $caseRoot 'existing.exe'
    [IO.File]::WriteAllText($existing, 'existing')
    $archiveBytes = [Text.Encoding]::UTF8.GetBytes('inert archive bytes')
    $archiveDigest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($archiveBytes)).ToLowerInvariant()
    $manifestBody = Manifest $archiveDigest
    if ($Scenario -eq 'malformed') { $manifestBody = 'not-a-manifest' }
    if ($Scenario -eq 'duplicate') { $manifestBody += "$archiveDigest  $selected`n" }
    if ($Scenario -eq 'mismatch') { $manifestBody = Manifest ('0' * 64) }
    if ($Scenario -eq 'uppercase_filename') {
        $manifestBody = $manifestBody.Replace($selected, $selected.ToUpperInvariant())
    }
    $manifestBytes = [Text.Encoding]::UTF8.GetBytes($manifestBody)
    $script:requestCounts = @{}
    $script:expandCount = 0
    $script:installCount = 0

    $request = {
        param([Uri]$Uri)
        $key = $Uri.AbsoluteUri
        if (-not $script:requestCounts.ContainsKey($key)) { $script:requestCounts[$key] = 0 }
        $script:requestCounts[$key]++
        if ($Scenario -eq 'download_failure') { return @{ StatusCode = 500 } }
        if ($Scenario -eq 'missing') { return @{ StatusCode = 404 } }
        if ($Scenario -eq 'protocol_downgrade') { return @{ StatusCode = 302; Location = 'http://unsafe.invalid/file' } }
        if ($Scenario -eq 'missing_location') { return @{ StatusCode = 302; Location = $null } }
        if ($Scenario -eq 'redirect_limit') { return @{ StatusCode = 302; Location = '/again' } }
        if ($Uri.AbsolutePath -eq '/assets/SHA256SUMS') {
            return @{ StatusCode = 200; Bytes = $manifestBytes }
        }
        if ($Uri.Host -eq 'github.com') {
            if ($Uri.AbsolutePath.EndsWith('/SHA256SUMS')) {
                return @{ StatusCode = 302; Location = '/assets/SHA256SUMS' }
            }
            return @{ StatusCode = 302; Location = 'https://objects.example.invalid/archive' }
        }
        return @{ StatusCode = 200; Bytes = $archiveBytes }
    }
    $download = {
        param([string]$Uri, [string]$OutFile)
        Invoke-HttpsDownload $Uri $OutFile $request
    }
    $expand = {
        param([string]$ArchivePath, [string]$DestinationPath)
        $script:expandCount++
        if (-not (Test-Path $ArchivePath -PathType Leaf)) { throw 'archive missing at expansion boundary' }
        [IO.File]::WriteAllText((Join-Path $DestinationPath 'coven-code.exe'), 'inert executable fixture')
    }
    $install = {
        param([string]$Source)
        $script:installCount++
        if (-not (Test-Path $Source -PathType Leaf)) { throw 'extracted fixture missing at install boundary' }
    }
    $boundaries = [pscustomobject]@{ Download = $download; Expand = $expand; Install = $install }
    $failed = $false
    try { Download-And-Install '9.9.9' 'x86_64' $boundaries } catch { $failed = $true }
    try {
        if ($ShouldPass) {
            if ($failed -or $script:expandCount -ne 1 -or $script:installCount -ne 1) {
                throw "$Scenario did not complete the expected inert flow"
            }
        } else {
            if (-not $failed) { throw "$Scenario unexpectedly succeeded" }
            if ($script:expandCount -ne 0 -or $script:installCount -ne 0) {
                throw "$Scenario crossed the extraction or install boundary"
            }
        }
        if ([IO.File]::ReadAllText($existing) -ne 'existing') { throw "$Scenario changed the existing target" }
    } finally { Remove-Item $caseRoot -Recurse -Force -ErrorAction SilentlyContinue }
}

Invoke-IntegrationCase 'valid_redirects' $true
Invoke-IntegrationCase 'missing' $false
Invoke-IntegrationCase 'malformed' $false
Invoke-IntegrationCase 'duplicate' $false
Invoke-IntegrationCase 'mismatch' $false
Invoke-IntegrationCase 'uppercase_filename' $false
Invoke-IntegrationCase 'download_failure' $false
Invoke-IntegrationCase 'protocol_downgrade' $false
Invoke-IntegrationCase 'missing_location' $false
Invoke-IntegrationCase 'redirect_limit' $false

Write-Host 'install.ps1 checksum and inert integration fixtures passed'
