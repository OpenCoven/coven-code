import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const RELEASE_ARCHIVES = Object.freeze([
  'coven-code-linux-aarch64.tar.gz',
  'coven-code-linux-x86_64.tar.gz',
  'coven-code-macos-aarch64.tar.gz',
  'coven-code-macos-x86_64.tar.gz',
  'coven-code-windows-x86_64.zip',
]);

export function generateReleaseChecksums(releaseDir) {
  const entries = fs.readdirSync(releaseDir, { withFileTypes: true });
  const archiveNames = entries
    .filter((entry) => entry.isFile() && /\.(?:tar\.gz|zip)$/i.test(entry.name))
    .map((entry) => entry.name)
    .sort();

  const missing = RELEASE_ARCHIVES.filter((name) => !archiveNames.includes(name));
  const unexpected = archiveNames.filter((name) => !RELEASE_ARCHIVES.includes(name));
  if (missing.length > 0) throw new Error(`missing release archives: ${missing.join(', ')}`);
  if (unexpected.length > 0) throw new Error(`unexpected release archives: ${unexpected.join(', ')}`);

  const manifest = RELEASE_ARCHIVES.map((name) => {
    const bytes = fs.readFileSync(path.join(releaseDir, name));
    const digest = crypto.createHash('sha256').update(bytes).digest('hex');
    return `${digest}  ${name}`;
  }).join('\n') + '\n';
  fs.writeFileSync(path.join(releaseDir, 'SHA256SUMS'), manifest, 'utf8');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const releaseDir = process.argv[2];
  if (!releaseDir || process.argv.length !== 3) {
    console.error('usage: generate-release-checksums.mjs <release-directory>');
    process.exit(2);
  }
  try {
    generateReleaseChecksums(path.resolve(releaseDir));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}
