import { expect, it } from 'vitest';
import fixture from './fixture.json' with { type: 'json' };
import type { Snapshot } from '../src/shared/lib/types';
import { translator } from '../src/shared/lib/i18n';
import { miningTitle } from '../src/app/title';

it('uses confirmed progress and replaces stale titles for pause, idle, disconnect and logout', () => {
  const data = structuredClone(fixture) as Snapshot;
  const t = translator({});
  const title = () => miningTitle(data, true, t);
  expect(title()).toBe('70% Rust - Drops Miner');
  data.current_drop!.current_minutes = 59;
  expect(title()).toBe('70% Rust - Drops Miner');
  data.current_drop!.confirmed_at = null;
  expect(title()).toBe('Rust - Drops Miner');
  data.settings.mining_paused = true;
  expect(title()).toBe('Paused - Drops Miner');
  expect(miningTitle(data, false, t)).toBe('Disconnected - Drops Miner');
  data.settings.mining_paused = false;
  data.mining!.state = 'waiting_channel';
  expect(title()).toBe('Idle - Drops Miner');
  data.mining!.state = 'manual_watching';
  data.current_drop = null;
  expect(title()).toBe('Watching northwind - Drops Miner');
  data.login.user_id = null;
  expect(title()).toBe('Drops Miner');
  expect(miningTitle(null, false, t)).toBe('Drops Miner');
});
