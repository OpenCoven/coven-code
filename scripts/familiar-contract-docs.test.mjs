import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const docs = fs.readFileSync(path.join(repoRoot, 'docs', 'familiars.md'), 'utf8');

assert.match(
  docs,
  /^## Testing Familiar Contract adherence$/m,
  'familiars docs should include a dedicated Familiar Contract testing section',
);

for (const requiredText of [
  'https://github.com/OpenCoven/familiar-contract',
  'node validators/validate.js',
  'SOUL.md',
  'IDENTITY.md',
  'MEMORY.md',
  'ward.toml',
  'Named Identity',
  'Defined Purpose',
  'Bounded Authority',
  'Persistent Memory',
  'Human Belonging',
  'structural compliance',
  'behavioral compliance',
  '.github/workflows/familiar-contract.yml',
]) {
  assert.match(docs, new RegExp(requiredText.replaceAll('.', '\\.')), `docs should mention ${requiredText}`);
}

console.log('familiar contract docs test passed');
