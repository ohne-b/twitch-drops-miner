import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { readVersion } from './release.mjs';

// Run only the explicitly built offline fixture, never an installed or production app.
const production = process.argv[2];
const executable = resolve((production ?? 'target/debug/twitch-drops-miner-desktop') + (process.platform === 'win32' ? '.exe' : ''));
if (production) {
  const version = spawnSync(executable, ['--version'], { timeout: 10_000, encoding: 'utf8' });
  const refusal = spawnSync(executable, ['--offline-smoke'], { timeout: 10_000, encoding: 'utf8' });
  const preview = spawnSync(executable, ['--offline-preview'], { timeout: 10_000, encoding: 'utf8' });
  if (version.error || version.status !== 0 || version.stdout.trim() !== `Drops Miner ${readVersion()}` ||
      refusal.error || refusal.status !== 2 || preview.error || preview.status !== 2) {
    throw new Error('The production executable must report its version and reject the offline fixture.');
  }
  console.log('Production executable: correct version, offline fixture excluded.');
  process.exit(0);
}
const result = spawnSync(executable, ['--offline-smoke'], { timeout: 60_000, encoding: 'utf8' });
process.stdout.write(result.stdout ?? '');
process.stderr.write(result.stderr ?? '');
if (result.error || result.status !== 0 || !result.stdout?.includes('Native smoke passed:')) {
  throw new Error('The offline native smoke test failed or timed out.');
}
