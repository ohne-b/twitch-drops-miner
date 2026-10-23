import { expect, it } from 'vitest';
import { translator } from '../src/shared/lib/i18n';
import {
  countdownSeconds,
  formatCountdown,
  formatProgressTime,
  updateCountdown,
} from '../src/shared/lib/progress';

const t = translator({});

it.each([
  [0, 45, '0m / 45m'],
  [45, 60, '45m / 1h'],
  [60, 60, '1h / 1h'],
  [61, 90, '1h 1m / 1h 30m'],
  [330, 360, '5h 30m / 6h'],
  [1500, 1561, '25h / 26h 1m'],
  [0, 0, '0m / 0m'],
])('formats %i of %i minutes as %s', (current, total, expected) => {
  expect(formatProgressTime(current, total, 'hours_minutes', t)).toBe(expected);
});

it('preserves the original minutes format by default', () => {
  expect(formatProgressTime(330, 360, 'minutes', t)).toBe('330 / 360 min');
  expect(formatProgressTime(330, 360, undefined, t)).toBe('330 / 360 min');
});

it.each([
  [0, '00:00:00'],
  [59, '00:00:59'],
  [60, '00:01:00'],
  [9318, '02:35:18'],
  [360000, '100:00:00'],
])('formats %i remaining seconds as %s', (seconds, text) => {
  expect(formatCountdown(seconds)).toBe(text);
});

it('freezes on pause/disconnect and resumes without counting time away', () => {
  let clock = updateCountdown(null, 'account/drop', 120, true, 1000)!;
  expect(countdownSeconds(clock, 11000)).toBe(110);
  clock = updateCountdown(clock, 'account/drop', 120, false, 11000)!;
  expect(countdownSeconds(clock, 300000)).toBe(110);
  clock = updateCountdown(clock, 'account/drop', 120, true, 300000)!;
  expect(countdownSeconds(clock, 310000)).toBe(100);
  expect(countdownSeconds(clock, 900000)).toBe(61);
});

it('does not reset on unrelated updates and bounds hidden-tab elapsed time', () => {
  const clock = updateCountdown(null, 'a', 720, true, 0)!;
  expect(updateCountdown(clock, 'a', 720, true, 30000)).toBe(clock);
  expect(countdownSeconds(clock, 9999999)).toBe(661);
  expect(countdownSeconds(clock, -1000)).toBe(720);
});

it('accepts corrections and resets for new accounts, rewards and server instances', () => {
  const clock = updateCountdown(null, 'a', 120, true, 0)!;
  const corrected = updateCountdown(clock, 'a', 180, true, 30000)!;
  expect(countdownSeconds(corrected, 31000)).toBe(179);
  const switched = updateCountdown(clock, 'b', 120, false, 30000)!;
  expect(countdownSeconds(switched, 60000)).toBe(120);
  expect(updateCountdown(clock, 'a', null, true, 0)).toBeNull();
  expect(updateCountdown(clock, 'a', NaN, true, 0)).toBeNull();
  expect(updateCountdown(clock, 'a', -1, true, 0)).toBeNull();
});

it('clamps estimates at zero without changing confirmed progress', () => {
  const clock = updateCountdown(null, 'a', 30, true, 0)!;
  expect(countdownSeconds(clock, 100000)).toBe(0);
});
