param([Parameter(Mandatory)][string]$ReleaseDir)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repo = Split-Path $PSScriptRoot
$name = 'coven-code-windows-x86_64.zip'
$archive = Join-Path $ReleaseDir $name
$root = Join-Path $env:RUNNER_TEMP "Coven QA/O'Brien"
$prefix = Join-Path $root 'prefix'
$tmp = Join-Path $root 'temp'
$source = Join-Path $root 'npm'
New-Item -ItemType Directory -Path $root,$tmp,$prefix | Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'npm') -Destination $source -Recurse
Push-Location $root
try {
    $env:TEMP = $tmp
    $env:TMP = $tmp
    $expected = (Get-FileHash -LiteralPath $archive).Hash.ToLowerInvariant()
    $manifest = @{ $name = @{ sha256 = $expected } }
    $manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $source 'checksums.json') -Encoding utf8NoBOM
    node (Join-Path $repo 'scripts/prepare-npm-package.mjs') --root $root --name '@opencoven/coven-code' --version 0.9.3
    if ($LASTEXITCODE) { throw "Package preparation failed: $LASTEXITCODE" }
    $packed = npm.cmd pack $source --json | ConvertFrom-Json
    if ($LASTEXITCODE -or $packed.name -ne '@opencoven/coven-code' -or $packed.version -ne '0.9.3') { throw 'Unexpected packed identity' }
    $tarball = Join-Path $root $packed.filename
    $package = Join-Path $prefix 'node_modules/@opencoven/coven-code'
    $exe = Join-Path $package 'native/coven-code.exe'
    $referenceDir = Join-Path $root 'reference'
    Expand-Archive -LiteralPath $archive -DestinationPath $referenceDir
    $referenceHash = (Get-FileHash -LiteralPath (Join-Path $referenceDir 'coven-code.exe')).Hash
    Write-Host "Package $($packed.name)@$($packed.version); reference executable SHA-256 $referenceHash"
    npm.cmd install --global --prefix $prefix --foreground-scripts --no-audit --no-fund $tarball
    if ($LASTEXITCODE) { throw "npm install exited $LASTEXITCODE" }
    $hash = (Get-FileHash -LiteralPath $exe).Hash
    if ($hash -ne $referenceHash) { throw 'Installed executable differs from verified archive' }
    $version = & (Join-Path $prefix 'coven-code.cmd') --version
    $code = $LASTEXITCODE
    Write-Host "Installed executable $exe; SHA-256 $hash; launcher output $version; exit $code"
    if ($code -ne 0 -or "$version" -notmatch '\b0\.9\.3\b') { throw 'Installed launcher failed version check' }
} finally {
    Pop-Location
}
