import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const PACKAGE_NAMES = Object.freeze(['@opencoven/coven-code']);

function assertSupportedPackageName(packageName) {
  if (!PACKAGE_NAMES.includes(packageName)) {
    throw new Error(`unsupported npm package name: ${packageName}`);
  }
}

function assertVersion(version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`invalid npm package version: ${version}`);
  }
}

function replaceRequired(text, pattern, replacement, label) {
  if (!pattern.test(text)) {
    throw new Error(`failed to update ${label}`);
  }
  const next = text.replace(pattern, replacement);
  return next;
}

function stringifyJson(value) {
  return JSON.stringify(value, null, 2).replace(/[^\x00-\x7F]/g, (char) => {
    return `\\u${char.codePointAt(0).toString(16).padStart(4, '0')}`;
  }) + '\n';
}

function updateReadme(readme, packageName) {
  let next = readme;
  next = replaceRequired(next, /^# .+$/m, `# ${packageName}`, 'README heading');
  next = replaceRequired(
    next,
    /npm\/v\/[^?]+(\?style=flat-square)/,
    `npm/v/${packageName}$1`,
    'README npm badge'
  );
  next = replaceRequired(
    next,
    /npmjs\.com\/package\/[^\)\s]+/,
    `npmjs.com/package/${packageName}`,
    'README npm package link'
  );
  next = replaceRequired(
    next,
    /^npm install -g .+$/m,
    `npm install -g ${packageName}`,
    'README npm install command'
  );
  next = replaceRequired(
    next,
    /^bun install -g .+$/m,
    `bun install -g ${packageName}`,
    'README bun install command'
  );
  return next;
}

export function publishArgsForPackage(packageName) {
  assertSupportedPackageName(packageName);
  return ['publish', '--access', 'public', '--provenance'];
}

export function prepareNpmPackage({ rootDir, packageName, version }) {
  assertSupportedPackageName(packageName);
  assertVersion(version);

  const npmDir = path.join(rootDir, 'npm');
  const packageJsonPath = path.join(npmDir, 'package.json');
  const readmePath = path.join(npmDir, 'README.md');

  const pkg = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
  pkg.name = packageName;
  pkg.version = version;
  fs.writeFileSync(packageJsonPath, stringifyJson(pkg));

  const readme = fs.readFileSync(readmePath, 'utf8');
  fs.writeFileSync(readmePath, updateReadme(readme, packageName), 'utf8');
}

function parseArgs(argv) {
  const args = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key || !key.startsWith('--') || !value) {
      throw new Error('usage: prepare-npm-package.mjs --name <package> --version <x.y.z> [--root <path>]');
    }
    const normalizedKey = key.slice(2);
    if (!['name', 'version', 'root'].includes(normalizedKey)) {
      throw new Error(`unsupported argument: ${key}`);
    }
    args.set(normalizedKey, value);
  }
  return {
    packageName: args.get('name'),
    version: args.get('version'),
    rootDir: args.get('root'),
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { packageName, version, rootDir } = parseArgs(process.argv.slice(2));
  prepareNpmPackage({
    rootDir: rootDir || path.dirname(path.dirname(fileURLToPath(import.meta.url))),
    packageName,
    version,
  });
}
