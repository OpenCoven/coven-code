import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'coven-launcher-'));
try {
  fs.mkdirSync(path.join(fixture, 'bin'));
  fs.mkdirSync(path.join(fixture, 'native'));
  const launcher = path.join(fixture, 'bin/coven-code');
  const binary = path.join(fixture, 'native/coven-code');
  fs.copyFileSync(path.join(root, 'npm/bin/coven-code'), launcher);
  const run = (...args) => spawnSync(process.execPath, [launcher, ...args], {
    encoding: 'utf8', timeout: 5000,
    env: { ...process.env, COVEN_LAUNCHER_TEST: 'inherited' },
  });

  const missing = run();
  assert.equal(missing.status, 1);
  assert.match(missing.stderr, /native binary not found/);
  assert.ok(missing.stderr.includes(binary));

  fs.writeFileSync(binary, '#!/bin/sh\nprintf "%s\\n" "$1" "$COVEN_LAUNCHER_TEST"\nexit 7\n', { mode: 0o755 });
  const success = run('argument with spaces');
  assert.equal(success.status, 7);
  assert.equal(success.stdout, 'argument with spaces\ninherited\n');
  assert.equal(success.stderr, '');

  fs.chmodSync(binary, 0o644);
  const denied = run();
  assert.equal(denied.status, 1);
  assert.match(denied.stderr, /EACCES/);
  assert.ok(denied.stderr.includes(binary));

  fs.writeFileSync(binary, '#!/nonexistent/coven-test-interpreter\n');
  fs.chmodSync(binary, 0o755);
  const interpreter = run();
  assert.equal(interpreter.status, 1);
  assert.match(interpreter.stderr, /ENOENT/);
  assert.ok(interpreter.stderr.includes(binary));
} finally {
  fs.rmSync(fixture, { recursive: true, force: true });
}
