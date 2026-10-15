import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const mock = `
const https = require('https');
const fs = require('fs');
const { EventEmitter } = require('events');
const { Readable, PassThrough, Writable } = require('stream');
let calls = 0;
https.get = (url, callback) => {
  const request = new EventEmitter();
  process.nextTick(() => {
    calls++;
    if (process.env.SCENARIO === 'network') {
      request.emit('error', new Error('fixture network failure'));
      return;
    }
    if (calls > 30) {
      request.emit('error', new Error('fixture redirect overflow'));
      return;
    }
    let response;
    if (process.env.SCENARIO === 'abort') {
      response = new PassThrough();
      response.statusCode = 200;
      response.headers = {};
      callback(response);
      setImmediate(() => response.destroy(new Error('fixture response aborted')));
      return;
    }
    response = Readable.from(fs.readFileSync(process.env.FIXTURE_ARCHIVE));
    response.statusCode = 200;
    response.headers = {};
    if (process.env.SCENARIO === 'http') response.statusCode = 503;
    if (process.env.SCENARIO === 'loop' || (process.env.SCENARIO === 'redirect' && calls === 1)) {
      response.statusCode = 302;
      response.headers.location = '/redirected.tar.gz';
    }
    if (process.env.SCENARIO === 'insecure') {
      response.statusCode = 302;
      response.headers.location = 'http://example.invalid/archive';
    }
    callback(response);
  });
  return request;
};
if (process.env.SCENARIO === 'write') {
  fs.createWriteStream = () => new Writable({
    write(chunk, encoding, callback) { callback(new Error('fixture write failure')); }
  });
}
if (process.env.SCENARIO === 'extract') {
  require('child_process').execFileSync = () => { throw new Error('fixture extraction failure'); };
}
`;

for (const [scenario, expected] of [
  ['success', null], ['redirect', null], ['checksum', /Checksum mismatch/],
  ['abort', /fixture response aborted/], ['write', /fixture write failure/],
  ['network', /fixture network failure/], ['http', /HTTP 503/],
  ['loop', /Too many redirects/], ['insecure', /non-HTTPS/],
  ['extract', /fixture extraction failure/],
]) {
  test(scenario, () => {
    const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'coven-install-fixture-'));
    try {
      const pkg = path.join(temp, 'package');
      const stage = path.join(temp, 'stage');
      const downloads = path.join(temp, 'downloads');
      for (const dir of [pkg, stage, downloads]) fs.mkdirSync(dir);
      fs.copyFileSync(path.join(root, 'npm/install.js'), path.join(pkg, 'install.js'));
      fs.writeFileSync(path.join(pkg, 'package.json'), JSON.stringify({ version: '1.2.3' }));
      fs.writeFileSync(path.join(stage, 'coven-code'), 'fixture native binary');
      const archive = path.join(temp, 'fixture.tar.gz');
      const tar = spawnSync('tar', ['-czf', archive, '-C', stage, 'coven-code']);
      assert.equal(tar.status, 0, tar.stderr.toString());
      const hash = scenario === 'checksum' ? '0'.repeat(64)
        : crypto.createHash('sha256').update(fs.readFileSync(archive)).digest('hex');
      const platform = process.platform === 'darwin' ? 'macos' : process.platform;
      const arch = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
      fs.writeFileSync(path.join(pkg, 'checksums.json'), JSON.stringify({
        [`coven-code-${platform}-${arch}.tar.gz`]: { sha256: hash },
      }));
      const preload = path.join(temp, 'mock.cjs');
      fs.writeFileSync(preload, mock);
      const result = spawnSync(process.execPath, ['--require', preload, path.join(pkg, 'install.js')], {
        env: { ...process.env, TMPDIR: downloads, SCENARIO: scenario, FIXTURE_ARCHIVE: archive },
        encoding: 'utf8', timeout: 5000,
      });
      assert.ifError(result.error);
      if (expected) {
        assert.equal(result.status, 1, result.stdout + result.stderr);
        assert.match(result.stderr, /coven-code install failed:/);
        assert.match(result.stderr, expected);
      } else {
        assert.equal(result.status, 0, result.stdout + result.stderr);
        const binary = path.join(pkg, 'native/coven-code');
        assert.equal(fs.readFileSync(binary, 'utf8'), 'fixture native binary');
        assert.ok(fs.statSync(binary).mode & 0o111);
      }
      assert.deepEqual(fs.readdirSync(downloads), [], 'temporary downloads leaked');
    } finally {
      fs.rmSync(temp, { recursive: true, force: true });
    }
  });
}
