import { createHash } from 'node:crypto';
import { copyFileSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { readVersion, releaseManifest, validateVersion } from './release.mjs';
import { run, validationSource } from './validated-artifacts.mjs';

export const targets = ['windows-x64', 'macos-universal', 'linux-x64'];
export function assets(version, target) {
  validateVersion(version);
  const prefix = `twitch-drops-miner-${version.replace('+', '_')}`;
  const formats = {
    'windows-x64': [['nsis', '.exe', '-windows-x64-setup.exe', ['windows-x86_64-nsis']]],
    'macos-universal': [
      ['dmg', '.dmg', '-macos.dmg', []],
      ['macos', '.app.tar.gz', '-macos-universal.app.tar.gz', ['darwin-x86_64-app', 'darwin-aarch64-app']],
    ],
    'linux-x64': [
      ['appimage', '.AppImage', '-linux-x64.AppImage', ['linux-x86_64-appimage']],
      ['deb', '.deb', '-linux-x64.deb', ['linux-x86_64-deb']],
    ],
  };
  if (!formats[target]) throw new Error('Unsupported desktop target.');
  return formats[target].map(([folder, extension, suffix, platforms]) => ({ folder, extension, name: prefix + suffix, platforms }));
}

export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const contents = file => {
  const stat = lstatSync(file);
  if (!stat.isFile() || stat.size < 1 || stat.size > 512 * 1024 * 1024) throw new Error('Invalid desktop artifact.');
  return readFileSync(file);
};

export function packDesktop(target, bundle, output, sha = process.env.GITHUB_SHA, version = readVersion()) {
  if (!/^[a-f0-9]{40}$/.test(sha ?? '')) throw new Error('A commit SHA is required for desktop artifacts.');
  mkdirSync(output, { recursive: true });
  const files = assets(version, target).map(asset => {
    const directory = join(bundle, asset.folder);
    const candidates = readdirSync(directory).filter(name => name.endsWith(asset.extension));
    if (candidates.length !== 1) throw new Error(`Expected one ${asset.extension} bundle.`);
    const source = join(directory, candidates[0]);
    const bytes = contents(source);
    copyFileSync(source, join(output, asset.name));
    return { name: asset.name, sha256: sha256(bytes), size: bytes.length };
  });
  writeFileSync(join(output, 'build.json'), JSON.stringify({ version, sha, target, files }, null, 2) + '\n');
}

export function verifyDesktop(directory, target, version, sha) {
  const metadata = JSON.parse(readFileSync(join(directory, 'build.json'), 'utf8'));
  const expected = assets(version, target);
  if (metadata.version !== version || metadata.sha !== sha || metadata.target !== target ||
      !Array.isArray(metadata.files) || metadata.files.length !== expected.length) throw new Error('Desktop artifact identity mismatch.');
  return expected.map(asset => {
    const records = metadata.files.filter(file => file.name === asset.name);
    if (records.length !== 1) throw new Error('Missing or duplicate desktop artifact.');
    const file = join(directory, asset.name);
    const bytes = contents(file);
    if (bytes.length !== records[0].size || sha256(bytes) !== records[0].sha256) throw new Error('Desktop artifact checksum mismatch.');
    return { ...asset, file };
  });
}

export function prepareDesktop(version) {
  if (readVersion() !== validateVersion(version)) throw new Error('Desktop version differs from Cargo.');
  const source = validationSource();
  const directory = mkdtempSync(join(tmpdir(), 'tdm-desktop-'));
  source.download(targets.map(target => `desktop-${target}`), directory);
  // Validate the entire set before signing or publishing any platform.
  const files = targets.flatMap(target => verifyDesktop(join(directory, `desktop-${target}`), target, version, source.sha));
  source.check();
  const output = resolve('dist/release');
  mkdirSync(output, { recursive: true });
  const config = JSON.parse(readFileSync('desktop/tauri.conf.json', 'utf8'));
  const publicKey = join(directory, 'updater.pub');
  writeFileSync(publicKey, Buffer.from(config.plugins.updater.pubkey, 'base64'));
  if (!process.env.TAURI_SIGNING_PRIVATE_KEY) throw new Error('Configure the prod updater signing secret.');
  const platforms = {};
  for (const asset of files) {
    const file = join(output, asset.name);
    copyFileSync(asset.file, file);
    if (!asset.platforms.length) continue;
    run(process.execPath, ['frontend/node_modules/@tauri-apps/cli/tauri.js', 'signer', 'sign', '--app-version', version, file], { stdio: 'pipe' });
    const signature = readFileSync(file + '.sig', 'utf8').trim();
    const decoded = Buffer.from(signature, 'base64').toString('utf8');
    const signatureFile = join(directory, asset.name + '.minisig');
    writeFileSync(signatureFile, decoded);
    // Verify with the public key embedded in the tested app, including the trusted version comment.
    run('minisign', ['-Vm', file, '-p', publicKey, '-x', signatureFile], { stdio: 'pipe' });
    if (!decoded.split('\n').some(line => line.startsWith('trusted comment: ') && line.split('\t').includes(`version:${version}`))) {
      throw new Error('Updater signature has no matching signed version.');
    }
    for (const platform of asset.platforms) platforms[platform] = {
      url: `https://github.com/ohne-b/twitch-drops-miner/releases/download/v${version}/${asset.name}`, signature,
    };
  }
  writeFileSync(join(output, 'latest.json'), JSON.stringify(releaseManifest(version, platforms, new Date().toISOString()), null, 2) + '\n');
  const published = [...files.map(file => file.name), 'latest.json'];
  writeFileSync(join(output, 'SHA256SUMS'), published.map(name => `${sha256(readFileSync(join(output, name)))}  ${name}\n`).join(''));
  writeFileSync(join(output, 'assets.txt'), [...published, 'SHA256SUMS'].map(name => join(output, name)).join('\n') + '\n');
  source.check();
  console.log(`Prepared desktop artifacts from validation run ${source.id} (${source.sha}).`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [command, ...args] = process.argv.slice(2);
  if (command === 'pack') packDesktop(...args);
  else if (command === 'prepare') prepareDesktop(...args);
  else throw new Error('Use pack TARGET BUNDLE OUTPUT or prepare VERSION.');
}
