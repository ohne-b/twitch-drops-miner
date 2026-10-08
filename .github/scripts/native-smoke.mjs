import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

// Run only the explicitly built offline fixture, never an installed or production app.
const executable = resolve('target/debug/twitch-drops-miner-desktop' + (process.platform === 'win32' ? '.exe' : ''));
const result = spawnSync(executable, [], { timeout: 60_000, encoding: 'utf8' });
process.stdout.write(result.stdout ?? '');
process.stderr.write(result.stderr ?? '');
if (result.error || result.status !== 0 || !result.stdout?.includes('Native smoke passed:')) {
  throw new Error('The offline native smoke test failed or timed out.');
}
