import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const semver = createRequire(new URL('../../frontend/package.json', import.meta.url))('semver');

export function validateVersion(version, previous) {
  const parsed = typeof version === 'string' ? semver.parse(version) : null;
  const canonical = parsed && parsed.version + (parsed.build.length ? `+${parsed.build.join('.')}` : '');
  if (!parsed || canonical !== version || version.length > 128 ||
      (previous && !semver.gt(version, previous))) {
    throw new Error('Use a canonical SemVer version (up to 128 characters) greater than the current version.');
  }
  return version;
}

export const isPrerelease = version => semver.prerelease(validateVersion(version)) !== null;

const repository = 'https://github.com/ohne-b/twitch-drops-miner';

export function releaseImages(version) {
  const tag = validateVersion(version).replace('+', '_');
  return [`ghcr.io/ohne-b/twitch-drops-miner:${tag}`];
}

export function releaseManifest(version, platforms, pubDate) {
  validateVersion(version);
  if (platforms) {
    const keys = ['windows-x86_64-nsis', 'darwin-x86_64-app', 'darwin-aarch64-app', 'linux-x86_64-appimage', 'linux-x86_64-deb'];
    if (Object.keys(platforms).sort().join() !== keys.sort().join() || !pubDate || !Number.isFinite(Date.parse(pubDate))) {
      throw new Error('The updater manifest needs every desktop platform and a publication date.');
    }
    for (const { url, signature } of Object.values(platforms)) {
      const prefix = `${repository}/releases/download/v${version}/`;
      if (typeof url !== 'string' || !url.startsWith(prefix) || !/^[\w.+-]+$/.test(url.slice(prefix.length)) ||
          typeof signature !== 'string' || signature.length < 64 || signature.length > 4096 || !/^[A-Za-z0-9+/=]+$/.test(signature)) {
        throw new Error('Invalid desktop update URL or signature.');
      }
    }
  }
  return {
    schemaVersion: 1,
    version,
    notes: `[Check release notes on GitHub](${repository}/releases/tag/v${version})`,
    ...(platforms ? { pub_date: pubDate, platforms } : {}),
  };
}

export function releaseNotes(version, changelog) {
  validateVersion(version);
  const lines = changelog.split(/\r?\n/);
  const heading = `## [v${version}](${repository}/releases/tag/v${version})`;
  const matches = lines.flatMap((line, index) => line.startsWith(heading + ' ') ? [index] : []);
  if (matches.length !== 1) throw new Error(`Add exactly one CHANGELOG entry for v${version}.`);
  const start = matches[0] + 1;
  const next = lines.findIndex((line, index) => index >= start && line.startsWith('## '));
  const notes = lines.slice(start, next < 0 ? undefined : next).join('\n').trim();
  if (!notes.startsWith('- ')) throw new Error('Release notes must start with a change list.');
  const previous = next < 0 ? null : lines[next].match(/^## \[v([^\]]+)\]/)?.[1];
  if (previous) validateVersion(version, validateVersion(previous));
  const changes = previous ? `compare/v${previous}...v${version}` : `commits/v${version}`;
  return `${notes}\n\nChangelog: ${repository}/${changes}\n\n` +
    `Please [open an issue](${repository}/issues/new) to report bugs or request features.\n`;
}

export function writeReleaseArtifacts(version, directory = process.cwd()) {
  if (readVersion(directory) !== validateVersion(version)) throw new Error('Release differs from Cargo version.');
  const notes = releaseNotes(version, readFileSync(resolve(directory, 'CHANGELOG.md'), 'utf8'));
  const output = resolve(directory, 'dist/release');
  mkdirSync(output, { recursive: true });
  writeFileSync(resolve(output, 'latest.json'), JSON.stringify(releaseManifest(version), null, 2) + '\n');
  writeFileSync(resolve(output, 'notes.md'), notes);
}

export function readVersion(directory = process.cwd(), locked = true) {
  const args = ['metadata', '--format-version', '1'];
  if (locked) args.push('--locked');
  const metadata = JSON.parse(execFileSync('cargo', args, {
    cwd: directory, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'],
  }));
  const pkg = metadata.packages.find(p => p.name === 'twitch-drops-miner' && metadata.workspace_members.includes(p.id));
  if (!pkg) throw new Error('The twitch-drops-miner package is missing.');
  if (metadata.packages.some(p => metadata.workspace_members.includes(p.id) && p.version !== pkg.version)) throw new Error('Workspace versions must match.');
  return validateVersion(pkg.version);
}

export function bumpVersion(version, directory = process.cwd()) {
  validateVersion(version, readVersion(directory));
  const manifest = resolve(directory, 'Cargo.toml');
  const lock = resolve(directory, 'Cargo.lock');
  const original = readFileSync(manifest, 'utf8');
  const previousLock = readFileSync(lock);
  // Cargo owns TOML parsing and lockfile generation. Only edit the shared workspace version.
  const pattern = /(^\[workspace\.package\]\r?\n[\s\S]*?^version\s*=\s*")[^"]+("\s*$)/m;
  if (!pattern.test(original)) throw new Error('Workspace package version is missing.');
  try {
    writeFileSync(manifest, original.replace(pattern, (_, before, after) => `${before}${version}${after}`));
    readVersion(directory, false);
    if (readVersion(directory) !== version) throw new Error('Version update did not persist.');
  } catch (error) {
    writeFileSync(manifest, original);
    writeFileSync(lock, previousLock);
    throw error;
  }
  return version;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [command, version] = process.argv.slice(2);
  if (command === 'read') console.log(readVersion());
  else if (command === 'bump') console.log(bumpVersion(version));
  else if (command === 'prerelease') console.log(isPrerelease(version));
  else if (command === 'images') console.log(releaseImages(version).join('\n'));
  else if (command === 'artifacts') writeReleaseArtifacts(version);
  else if (command === 'verify') {
    validateVersion(version);
    if (readVersion() !== version) throw new Error('Requested release differs from the checked-out package.');
    console.log(version);
  } else throw new Error('Usage: node .github/scripts/release.mjs read|bump VERSION|verify VERSION|prerelease VERSION|images VERSION|artifacts VERSION');
}
