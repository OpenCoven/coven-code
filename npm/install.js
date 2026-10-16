#!/usr/bin/env node
'use strict';

const https = require('https');
const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');
const { execFileSync } = require('child_process');
const { pipeline } = require('stream/promises');

const pkg = require('./package.json');
const checksums = require('./checksums.json');
const VERSION = pkg.version;
const REPO = 'OpenCoven/coven-code';
const BASE_URL = `https://github.com/${REPO}/releases/download/v${VERSION}`;
const NATIVE_DIR = path.join(__dirname, 'native');
const DOWNLOAD_TIMEOUT_MS = 30_000;

function getPlatform() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === 'win32' && arch === 'x64') {
    return { artifact: 'coven-code-windows-x86_64', ext: '.exe', archive: '.zip' };
  }
  if (platform === 'linux' && arch === 'x64') {
    return { artifact: 'coven-code-linux-x86_64', ext: '', archive: '.tar.gz' };
  }
  if (platform === 'linux' && arch === 'arm64') {
    return { artifact: 'coven-code-linux-aarch64', ext: '', archive: '.tar.gz' };
  }
  if (platform === 'darwin' && arch === 'x64') {
    return { artifact: 'coven-code-macos-x86_64', ext: '', archive: '.tar.gz' };
  }
  if (platform === 'darwin' && arch === 'arm64') {
    return { artifact: 'coven-code-macos-aarch64', ext: '', archive: '.tar.gz' };
  }
  throw new Error(
    `Unsupported platform: ${platform}/${arch}.\n` +
    `Install manually from: https://github.com/${REPO}/releases/tag/v${VERSION}`
  );
}

async function download(url, dest, redirects = 0, deadline = Date.now() + DOWNLOAD_TIMEOUT_MS) {
  const parsed = new URL(url);
  if (parsed.protocol !== 'https:') {
    throw new Error(`Refusing to download non-HTTPS URL: ${url}`);
  }

  const remaining = deadline - Date.now();
  if (remaining <= 0) {
    throw new Error('Download timed out');
  }
  let request;
  let response;
  let timer;
  try {
    const res = await new Promise((resolve, reject) => {
      request = https.get(url, resolve).on('error', reject);
      timer = setTimeout(() => {
        const error = new Error('Download timed out');
        request.destroy(error);
        response?.destroy(error);
      }, remaining);
    });
    response = res;
    if ([301, 302, 303, 307, 308].includes(res.statusCode)) {
      res.destroy();
      if (redirects >= 5) {
        throw new Error(`Too many redirects downloading ${url}`);
      }
      if (!res.headers.location) {
        throw new Error(`Redirect without Location header downloading ${url}`);
      }
      const location = new URL(res.headers.location, url).toString();
      return download(location, dest, redirects + 1, deadline);
    }
    if (res.statusCode !== 200) {
      res.destroy();
      throw new Error(`HTTP ${res.statusCode} downloading ${url}`);
    }
    await pipeline(res, fs.createWriteStream(dest));
  } finally {
    clearTimeout(timer);
  }
}

function sha256File(filePath) {
  const hash = crypto.createHash('sha256');
  const bytes = fs.readFileSync(filePath);
  hash.update(bytes);
  return hash.digest('hex');
}

function expectedSha256(archiveName) {
  const entry = checksums[archiveName];
  if (!entry || typeof entry.sha256 !== 'string') {
    throw new Error(`Missing SHA-256 checksum for ${archiveName} in checksums.json`);
  }
  return entry.sha256;
}

function verifyChecksum(filePath, archiveName) {
  const expected = expectedSha256(archiveName).toLowerCase();
  const actual = sha256File(filePath).toLowerCase();
  if (actual !== expected) {
    throw new Error(
      `Checksum mismatch for ${archiveName}: expected ${expected}, got ${actual}`
    );
  }
}

async function main() {
  const { artifact, ext, archive } = getPlatform();
  const archiveName = `${artifact}${archive}`;
  const url = `${BASE_URL}/${archiveName}`;
  const binaryDest = path.join(NATIVE_DIR, `coven-code${ext}`);

  if (fs.existsSync(binaryDest)) {
    console.log('coven-code: native binary already present, skipping download.');
    return;
  }

  fs.mkdirSync(NATIVE_DIR, { recursive: true });

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'coven-code-install-'));
  try {
    const tmpPath = path.join(tmpDir, archiveName);
    console.log(`coven-code: downloading v${VERSION} for ${process.platform}/${process.arch}`);
    console.log(`            ${url}`);
    await download(url, tmpPath);

    console.log('coven-code: verifying checksum...');
    verifyChecksum(tmpPath, archiveName);

    console.log('coven-code: extracting...');
    if (archive === '.zip') {
      execFileSync('powershell', [
        '-NoProfile', '-NonInteractive', '-Command',
        `Expand-Archive -Force -Path "${tmpPath}" -DestinationPath "${NATIVE_DIR}"`
      ]);
    } else {
      execFileSync('tar', ['-xzf', tmpPath, '-C', NATIVE_DIR]);
    }

    if (!fs.existsSync(binaryDest)) {
      throw new Error(`Extraction succeeded but binary not found at ${binaryDest}`);
    }

    if (ext === '') {
      fs.chmodSync(binaryDest, 0o755);
    }

    console.log(`coven-code: ready — run \`coven-code\` to start.`);
  } finally {
    try {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    } catch (err) {
      console.warn(`coven-code: could not remove temporary directory ${tmpDir}: ${err.message}`);
    }
  }
}

main().catch((err) => {
  console.error(`\ncoven-code install failed: ${err.message}`);
  console.error(`Manual install: https://github.com/${REPO}/releases/tag/v${VERSION}\n`);
  process.exit(1);
});
