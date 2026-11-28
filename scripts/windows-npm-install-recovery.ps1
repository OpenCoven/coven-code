param([Parameter(Mandatory)][string]$ReleaseDir)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$root = Join-Path $env:RUNNER_TEMP "Coven QA/O'Brien"
$name = 'coven-code-windows-x86_64.zip'
$archive = Join-Path $ReleaseDir $name
$preload = Join-Path $PSScriptRoot 'windows-npm-install-source.cjs'
$referenceHash = (Get-FileHash -LiteralPath (Join-Path $root 'reference/coven-code.exe')).Hash
$tarball = Join-Path $root 'opencoven-coven-code-0.9.3.tgz'
$bad = Join-Path $root 'invalid.zip'
[IO.File]::WriteAllText($bad, 'fixture: invalid zip')
foreach ($case in 'checksum','extraction','timeout') {
    $prefix = Join-Path $root $case
    $tmp = Join-Path $prefix 'temp'
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    $env:TEMP = $tmp
    $env:TMP = $tmp
    npm.cmd install --global --prefix $prefix --ignore-scripts --no-audit --no-fund $tarball
    if ($LASTEXITCODE) { throw "Cannot stage packed package for $case" }
    $package = Join-Path $prefix 'node_modules/@opencoven/coven-code'
    $installer = Join-Path $package 'install.js'
    $checksums = Join-Path $package 'checksums.json'
    $native = Join-Path $package 'native'
    $exe = Join-Path $native 'coven-code.exe'
    $saved = [IO.File]::ReadAllBytes($checksums)
    $env:INSTALL_CASE = $case
    $env:INSTALL_ARCHIVE = $bad
    if ($case -eq 'extraction') {
        $fixtureHash = (Get-FileHash -LiteralPath $bad).Hash.ToLowerInvariant()
        @{ $name = @{ sha256 = $fixtureHash } } | ConvertTo-Json | Set-Content -LiteralPath $checksums -Encoding utf8NoBOM
    }
    $output = & node --require $preload $installer 2>&1
    $code = $LASTEXITCODE
    $diagnostic = $output -join [Environment]::NewLine
    $pattern = @{ checksum = 'Checksum mismatch'; extraction = 'Expand-Archive'; timeout = 'Download timed out' }[$case]
    $residue = @(Get-ChildItem -LiteralPath $tmp -Filter 'coven-code-install-*')
    $stage = @(Get-ChildItem -LiteralPath $native -Filter '.coven-code-stage-*')
    Write-Host "$case exit=$code installed=$(Test-Path -LiteralPath $exe) temp=$($residue.Count) stage=$($stage.Count)"
    Write-Host $diagnostic
    if ($code -ne 1 -or $diagnostic -notmatch $pattern -or (Test-Path -LiteralPath $exe) -or $residue.Count -or $stage.Count) { throw "Unexpected $case failure or cleanup" }
    [IO.File]::WriteAllBytes($checksums, $saved)
    $env:INSTALL_CASE = 'retry'
    $env:INSTALL_ARCHIVE = $archive
    & node --require $preload $installer
    if ($LASTEXITCODE -or (Get-FileHash -LiteralPath $exe).Hash -ne $referenceHash) { throw "Retry after $case failed" }
    Write-Host "$case retry exit=0 executable SHA-256=$referenceHash"
}
$env:INSTALL_CASE = 'offline'
$before = (Get-FileHash -LiteralPath $exe).Hash
& node --require $preload $installer
$code = $LASTEXITCODE
$after = (Get-FileHash -LiteralPath $exe).Hash
Write-Host "Offline reuse exit=$code before=$before after=$after"
if ($code -ne 0 -or $before -ne $after) { throw 'Offline reuse failed' }
