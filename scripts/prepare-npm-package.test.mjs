import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import {
  PACKAGE_NAMES,
  prepareNpmPackage,
  publishArgsForPackage,
} from './prepare-npm-package.mjs';

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

function makeFixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'coven-code-npm-'));
  fs.mkdirSync(path.join(dir, 'npm'));
  fs.copyFileSync(path.join(repoRoot, 'npm', 'package.json'), path.join(dir, 'npm', 'package.json'));
  fs.copyFileSync(path.join(repoRoot, 'npm', 'README.md'), path.join(dir, 'npm', 'README.md'));
  return dir;
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

{
  const root = makeFixture();

  prepareNpmPackage({
    rootDir: root,
    packageName: 'coven-code',
    version: '1.2.3',
  });

  const pkg = readJson(path.join(root, 'npm', 'package.json'));
  const readme = fs.readFileSync(path.join(root, 'npm', 'README.md'), 'utf8');

  assert.equal(pkg.name, 'coven-code');
  assert.equal(pkg.version, '1.2.3');
  assert.deepEqual(pkg.bin, {
    'coven-code': 'bin/coven-code',
    coven: 'bin/coven-code',
    'coven-cave': 'bin/coven-code',
  });
  assert.match(readme, /^# coven-code$/m);
  assert.match(readme, /npm\/v\/coven-code\?style=flat-square/);
  assert.match(readme, /npmjs\.com\/package\/coven-code/);
  assert.match(readme, /npm install -g coven-code/);
  assert.match(readme, /bun install -g coven-code/);
}

{
  const root = makeFixture();

  prepareNpmPackage({
    rootDir: root,
    packageName: '@opencoven/coven-code',
    version: '2.3.4',
  });

  const pkg = readJson(path.join(root, 'npm', 'package.json'));
  const readme = fs.readFileSync(path.join(root, 'npm', 'README.md'), 'utf8');

  assert.equal(pkg.name, '@opencoven/coven-code');
  assert.equal(pkg.version, '2.3.4');
  assert.match(readme, /^# @opencoven\/coven-code$/m);
  assert.match(readme, /npm install -g @opencoven\/coven-code/);
}

assert.deepEqual(PACKAGE_NAMES, ['@opencoven/coven-code', 'coven-code']);
assert.deepEqual(publishArgsForPackage('@opencoven/coven-code'), [
  'publish',
  '--access',
  'public',
  '--provenance',
]);
assert.deepEqual(publishArgsForPackage('coven-code'), ['publish', '--provenance']);
assert.throws(
  () => prepareNpmPackage({ rootDir: makeFixture(), packageName: 'other-package', version: '1.0.0' }),
  /unsupported npm package name/
);

{
  const root = makeFixture();
  const result = spawnSync(process.execPath, [
    path.join(repoRoot, 'scripts', 'prepare-npm-package.mjs'),
    '--name',
    'coven-code',
    '--version',
    '4.5.6',
    '--root',
    root,
  ]);

  assert.equal(result.status, 0, result.stderr.toString());
  const pkg = readJson(path.join(root, 'npm', 'package.json'));
  assert.equal(pkg.name, 'coven-code');
  assert.equal(pkg.version, '4.5.6');
}
