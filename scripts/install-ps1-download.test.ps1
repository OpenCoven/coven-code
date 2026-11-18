$ErrorActionPreference = 'Stop'
$env:COVEN_CODE_INSTALLER_TEST = '1'
. $env:DOWNLOAD_INSTALLER

# Runs the installer's own HTTP download against an in-process handler, so no
# request leaves the machine. Output is left uncaptured: the caller reads this
# script's stdout as the installer's.
$InstallDir = $env:DOWNLOAD_TARGET

$fixtureAssemblies = @(
    [System.Net.Http.HttpClient].Assembly.Location
    [System.Net.HttpStatusCode].Assembly.Location
    [System.Threading.Tasks.Task].Assembly.Location
    [System.IO.Stream].Assembly.Location
    [System.Uri].Assembly.Location
) | Select-Object -Unique
Add-Type -ReferencedAssemblies $fixtureAssemblies -TypeDefinition @'
using System;
using System.IO;
using System.Net;
using System.Net.Http;
using System.Threading;
using System.Threading.Tasks;

public sealed class DownloadFixtureHandler : HttpMessageHandler {
    private readonly byte[] manifest;
    private readonly byte[] archive;
    private readonly bool failArchiveBody;

    public DownloadFixtureHandler(byte[] manifest, byte[] archive, bool failArchiveBody) {
        this.manifest = manifest;
        this.archive = archive;
        this.failArchiveBody = failArchiveBody;
    }

    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) {
        bool isManifest = request.RequestUri.AbsolutePath.EndsWith("/SHA256SUMS", StringComparison.Ordinal);
        var response = new HttpResponseMessage(HttpStatusCode.OK);
        response.Content = new StreamContent(new FixtureBodyStream(isManifest ? manifest : archive, failArchiveBody && !isManifest));
        return Task.FromResult(response);
    }
}

// A body that arrives in small pieces and can fail after the first one,
// leaving a partly written file.
public sealed class FixtureBodyStream : Stream {
    private readonly byte[] body;
    private readonly bool failAfterFirstRead;
    private int position;

    public FixtureBodyStream(byte[] body, bool failAfterFirstRead) {
        this.body = body;
        this.failAfterFirstRead = failAfterFirstRead;
    }

    public override bool CanRead { get { return true; } }
    public override bool CanSeek { get { return false; } }
    public override bool CanWrite { get { return false; } }
    public override long Length { get { throw new NotSupportedException(); } }
    public override long Position {
        get { throw new NotSupportedException(); }
        set { throw new NotSupportedException(); }
    }
    public override void Flush() { }
    public override int Read(byte[] buffer, int offset, int count) {
        if (failAfterFirstRead && position > 0) { throw new IOException("fixture body failure"); }
        int length = Math.Min(Math.Min(count, 4), body.Length - position);
        Buffer.BlockCopy(body, position, buffer, offset, length);
        position += length;
        return length;
    }
    public override long Seek(long offset, SeekOrigin origin) { throw new NotSupportedException(); }
    public override void SetLength(long value) { throw new NotSupportedException(); }
    public override void Write(byte[] buffer, int offset, int count) { throw new NotSupportedException(); }
}
'@

$archiveBytes = [Text.Encoding]::UTF8.GetBytes('inert archive bytes')
$sha = [Security.Cryptography.SHA256]::Create()
try { $digest = ([BitConverter]::ToString($sha.ComputeHash($archiveBytes))).Replace('-', '').ToLowerInvariant() }
finally { $sha.Dispose() }
$names = @(
    'coven-code-linux-aarch64.tar.gz', 'coven-code-linux-x86_64.tar.gz',
    'coven-code-macos-aarch64.tar.gz', 'coven-code-macos-x86_64.tar.gz',
    'coven-code-windows-x86_64.zip'
)
$manifest = (($names | ForEach-Object {
    $hash = if ($_ -eq 'coven-code-windows-x86_64.zip') { $digest } else { '0' * 64 }
    "$hash  $_"
}) -join "`n") + "`n"

$handler = [DownloadFixtureHandler]::new(
    [Text.Encoding]::UTF8.GetBytes($manifest), $archiveBytes, $env:DOWNLOAD_BODY -eq 'fail')
$client = [System.Net.Http.HttpClient]::new($handler)
$boundaries = [pscustomobject]@{
    Download = {
        param([string]$Uri, [string]$OutFile)
        Invoke-HttpsDownload $Uri $OutFile $null ([TimeSpan]::FromSeconds(30)) $client
    }
    Expand = {
        param([string]$ArchivePath, [string]$DestinationPath)
        Copy-Item -LiteralPath $env:DOWNLOAD_SOURCE -Destination (Join-Path $DestinationPath 'coven-code.exe')
    }
    Install = { param([string]$Source) Install-Binary $Source }
}
try {
    Download-And-Install '9.9.9' 'x86_64' $boundaries
} finally {
    $client.Dispose()
}
