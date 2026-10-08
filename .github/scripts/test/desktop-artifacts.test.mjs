import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { assets, packDesktop, targets, verifyDesktop } from '../desktop-artifacts.mjs';
import { releaseManifest } from '../release.mjs';

test('the complete desktop set keeps one universal Mac installer and platform-specific updates', () => {
  const all = targets.flatMap(target => assets('1.7.0', target));
  assert.equal(all.length, 5);
  assert.equal(all.filter(asset => asset.extension === '.dmg').length, 1);
  assert.ok(all.every(asset => !asset.name.endsWith('.msi')));
  const platforms = Object.fromEntries(all.flatMap(asset => asset.platforms.map(key => [key, {
    url: `https://github.com/ohne-b/twitch-drops-miner/releases/download/v1.7.0/${asset.name}`,
    signature: Buffer.from('signature fixture'.repeat(8)).toString('base64'),
  }])));
  const manifest = releaseManifest('1.7.0', platforms, '2026-10-08T12:00:00Z');
  assert.equal(manifest.schemaVersion, 1);
  assert.match(manifest.notes, /release notes/);
  assert.equal(manifest.platforms['darwin-x86_64-app'].url, manifest.platforms['darwin-aarch64-app'].url);
  assert.notEqual(manifest.platforms['linux-x86_64-appimage'].url, manifest.platforms['linux-x86_64-deb'].url);
  assert.throws(() => releaseManifest('1.7.0', { ...platforms, extra: platforms['linux-x86_64-deb'] }, '2026-10-08'));
  assert.throws(() => releaseManifest('1.7.1', platforms, '2026-10-08'));
  assert.throws(() => releaseManifest('1.7.0', platforms, 'bad date'));
});

test('promotion rejects stale, modified, missing and duplicate installer artifacts', () => {
  const directory = mkdtempSync(join(tmpdir(), 'tdm-bundles-'));
  const sha = 'a'.repeat(40);
  try {
    const bundle = join(directory, 'bundle');
    mkdirSync(join(bundle, 'nsis'), { recursive: true });
    writeFileSync(join(bundle, 'nsis', 'Drops Miner.exe'), 'fixture bundle');
    const output = join(directory, 'artifacts');
    packDesktop('windows-x64', bundle, output, sha, '1.7.0');
    const verified = verifyDesktop(output, 'windows-x64', '1.7.0', sha);
    assert.equal(verified.length, 1);
    assert.throws(() => verifyDesktop(output, 'windows-x64', '1.7.0', 'b'.repeat(40)));
    assert.throws(() => verifyDesktop(output, 'windows-x64', '1.7.1', sha));
    writeFileSync(verified[0].file, 'tampered bytes');
    assert.throws(() => verifyDesktop(output, 'windows-x64', '1.7.0', sha));
    const metadata = JSON.parse(readFileSync(join(output, 'build.json'), 'utf8'));
    metadata.files.push(metadata.files[0]);
    writeFileSync(join(output, 'build.json'), JSON.stringify(metadata));
    assert.throws(() => verifyDesktop(output, 'windows-x64', '1.7.0', sha));
    writeFileSync(join(bundle, 'nsis', 'another.exe'), 'duplicate');
    assert.throws(() => packDesktop('windows-x64', bundle, output, sha, '1.7.0'));
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
