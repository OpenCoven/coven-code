import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const mock = `
const https = require('https');
const { EventEmitter } = require('events');
const { PassThrough } = require('stream');
const realSetTimeout = global.setTimeout;
global.setTimeout = (callback, delay, ...args) =>
  realSetTimeout(callback, Math.min(delay, 30), ...args);
https.get = (url, callback) => {
  const request = new EventEmitter();
  let response;
  const connection = setInterval(() => {}, 1000);
  request.destroy = (error) => {
    clearInterval(connection);
    if (response) response.destroy(error);
    else request.emit('error', error);
  };
  if (process.env.SCENARIO === 'body') {
    process.nextTick(() => {
      response = new PassThrough();
      response.statusCode = 200;
      response.headers = {};
      callback(response);
    });
  }
  return request;
};
`;

for (const scenario of ['headers', 'body']) {
  test(`installer bounds stalled download ${scenario}`, () => {
    const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'coven-install-deadline-'));
    try {
      const pkg = path.join(temp, 'package');
      fs.mkdirSync(pkg);
      fs.copyFileSync(path.join(root, 'npm/install.js'), path.join(pkg, 'install.js'));
      fs.writeFileSync(path.join(pkg, 'package.json'), JSON.stringify({ version: '1.2.3' }));
      fs.writeFileSync(path.join(pkg, 'checksums.json'), '{}');
      const preload = path.join(temp, 'mock.cjs');
      fs.writeFileSync(preload, mock);
      const result = spawnSync(process.execPath, ['--require', preload, path.join(pkg, 'install.js')], {
        env: { ...process.env, SCENARIO: scenario, TMPDIR: temp },
        encoding: 'utf8',
        timeout: 3000,
      });
      assert.ifError(result.error);
      assert.equal(result.status, 1, result.stdout + result.stderr);
      assert.match(result.stderr, /Download timed out/);
      assert.deepEqual(fs.readdirSync(temp).sort(), ['mock.cjs', 'package']);
    } finally {
      fs.rmSync(temp, { recursive: true, force: true });
    }
  });
}
