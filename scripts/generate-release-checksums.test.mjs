import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const generator = path.join(root, 'scripts', 'generate-release-checksums.mjs');
const names = [
  'coven-code-linux-aarch64.tar.gz',
  'coven-code-linux-x86_64.tar.gz',
  'coven-code-macos-aarch64.tar.gz',
  'coven-code-macos-x86_64.tar.gz',
  'coven-code-windows-x86_64.zip',
];

function fixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'coven-checksums-'));
  for (const name of names) fs.writeFileSync(path.join(dir, name), `fixture:${name}\n`);
  return dir;
}

function run(dir) {
  return spawnSync(process.execPath, [generator, dir], { encoding: 'utf8' });
}

{
  const dir = fixture();
  const result = run(dir);
  assert.equal(result.status, 0, result.stderr);
  const expected = names.map((name) => {
    const bytes = fs.readFileSync(path.join(dir, name));
    return `${crypto.createHash('sha256').update(bytes).digest('hex')}  ${name}`;
  }).join('\n') + '\n';
  assert.equal(fs.readFileSync(path.join(dir, 'SHA256SUMS'), 'utf8'), expected);
  assert.equal(run(dir).status, 0);
}

{
  const dir = fixture();
  fs.rmSync(path.join(dir, names[0]));
  const result = run(dir);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /missing/i);
}

{
  const dir = fixture();
  fs.writeFileSync(path.join(dir, 'coven-code-linux-riscv64.tar.gz'), 'unexpected');
  const result = run(dir);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /unexpected/i);
}

{
  const dir = fixture();
  fs.writeFileSync(path.join(dir, 'unexpected.zip'), 'unexpected');
  const result = run(dir);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /unexpected/i);
}

for (const unexpected of ['unexpected.ZIP', 'unexpected.TaR.Gz']) {
  const dir = fixture();
  fs.writeFileSync(path.join(dir, unexpected), 'unexpected');
  const result = run(dir);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /unexpected/i);
}
