import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

function exists(relativePath) {
  return fs.existsSync(path.join(repoRoot, relativePath));
}

function read(relativePath) {
  return fs.readFileSync(path.join(repoRoot, relativePath), 'utf8');
}

assert.equal(exists('src-rust/crates/tui/src/mascot.rs'), true, 'mascot module file should exist');
assert.equal(exists('src-rust/crates/tui/src/rustle.rs'), false, 'legacy rustle module file should be removed');
assert.match(read('src-rust/crates/tui/src/lib.rs'), /pub mod mascot;/, 'tui lib should export mascot module');
assert.doesNotMatch(read('src-rust/crates/tui/src/lib.rs'), /pub mod rustle;/, 'tui lib should not export rustle module');

for (const file of [
  'src-rust/crates/tui/src/mascot.rs',
  'src-rust/crates/tui/src/familiar_card.rs',
  'src-rust/crates/tui/src/app.rs',
  'src-rust/crates/tui/src/render.rs',
  'src-rust/crates/cli/src/main.rs',
]) {
  const source = read(file);
  assert.doesNotMatch(
    source,
    /RustlePose|rustle_lines_for|crate::rustle|tick_rustle_pose|rustle_current_pose|rustle_look_down/,
    `${file} should not use legacy Rustle identifiers`,
  );
}

assert.match(read('src-rust/crates/tui/src/mascot.rs'), /pub enum CompanionPose/, 'pose type should be CompanionPose');
assert.match(read('src-rust/crates/tui/src/mascot.rs'), /pub fn mascot_lines_for/, 'legacy rustle_lines_for should be renamed');

assert.equal(exists('public/Rune.png'), true, 'Rune mascot asset should exist');
assert.equal(exists('public/Pirate-Rune.png'), true, 'Pirate Rune asset should exist');
assert.equal(exists('public/Rustle.png'), false, 'legacy Rustle asset should be removed');
assert.equal(exists('public/Pirate-Rustle.png'), false, 'legacy Pirate Rustle asset should be removed');

console.log('mascot rebrand smoke test passed');
