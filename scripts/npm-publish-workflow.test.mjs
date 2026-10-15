import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const workflow = fs.readFileSync(path.join(root, '.github/workflows/npm-publish.yml'), 'utf8');
function stepScript(name) {
  const step = workflow.split(`      - name: ${name}\n`)[1]?.split('\n      - ')[0];
  assert.ok(step, `missing workflow step: ${name}`);
  const body = step.split('        run: |\n')[1];
  assert.ok(body, `missing shell body: ${name}`);
  return body.split('\n').map(line => line.startsWith('          ') ? line.slice(10) : line).join('\n');
}

const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'coven-publish-workflow-'));
try {
  const source = path.join(fixture, 'tagged source');
  const runner = path.join(fixture, 'runner temp');
  const bin = path.join(fixture, 'bin');
  const runnerStorage = path.join(fixture, 'runner storage');
  for (const dir of [path.join(source, 'npm'), runnerStorage, bin]) fs.mkdirSync(dir, { recursive: true });
  fs.symlinkSync(runnerStorage, runner, 'dir');
  for (const name of ['package.json', 'README.md']) {
    fs.copyFileSync(path.join(root, 'npm', name), path.join(source, 'npm', name));
  }
  // The tagged source intentionally has no scripts directory. The workflow
  // must obtain its helper from its own commit, not this release snapshot.
  const sha = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' });
  assert.equal(sha.status, 0, sha.stderr);
  const log = path.join(fixture, 'npm.log');
  fs.writeFileSync(path.join(bin, 'npm'), `#!/bin/bash
printf '%s\\n' "$*" >> "$NPM_LOG"
case "$1" in
  view) test "$ALREADY_PUBLISHED" = 1 ;;
  publish) exit 0 ;;
  *) exit 90 ;;
esac
`, { mode: 0o755 });
  const env = {
    ...process.env, WORKFLOW_SHA: sha.stdout.trim(), RUNNER_TEMP: runner,
    GITHUB_WORKSPACE: source, TMPDIR: runner, VERSION: '12.34.56', NPM_LOG: log,
    PATH: `${bin}${path.delimiter}${process.env.PATH}`, NODE_AUTH_TOKEN: '', ALREADY_PUBLISHED: '0',
  };
  const staged = spawnSync('bash', ['-e', '-c', stepScript('Stage packaging helper')], {
    cwd: root, env, encoding: 'utf8', timeout: 10000,
  });
  assert.equal(staged.status, 0, staged.stderr);
  assert.equal(
    fs.readFileSync(path.join(runner, 'prepare-npm-package.mjs'), 'utf8'),
    spawnSync('git', ['show', `${env.WORKFLOW_SHA}:scripts/prepare-npm-package.mjs`], {
      cwd: root, encoding: 'utf8',
    }).stdout,
  );
  // Exercise uncommitted helper edits too, after verifying the git-show stage.
  fs.copyFileSync(path.join(root, 'scripts/prepare-npm-package.mjs'), path.join(runner, 'prepare-npm-package.mjs'));
  const publish = () => spawnSync('bash', ['-e', '-c', stepScript('Publish npm packages')], {
    cwd: source, env, encoding: 'utf8', timeout: 10000,
  });
  const result = publish();
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.equal(JSON.parse(fs.readFileSync(path.join(source, 'npm/package.json'), 'utf8')).version, '12.34.56');
  assert.deepEqual(fs.readFileSync(log, 'utf8').trim().split('\n'), [
    'view @opencoven/coven-code@12.34.56 version',
    'publish --access public --provenance --ignore-scripts',
  ]);
  env.ALREADY_PUBLISHED = '1';
  fs.writeFileSync(log, '');
  const before = fs.readFileSync(path.join(source, 'npm/package.json'));
  const skipped = publish();
  assert.equal(skipped.status, 0, skipped.stdout + skipped.stderr);
  assert.equal(fs.readFileSync(log, 'utf8').trim(), 'view @opencoven/coven-code@12.34.56 version');
  assert.deepEqual(fs.readFileSync(path.join(source, 'npm/package.json')), before);
} finally {
  fs.rmSync(fixture, { recursive: true, force: true });
}
