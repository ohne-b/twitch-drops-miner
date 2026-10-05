import { expect, it } from 'vitest';
import { readableNameColor } from '../src/features/account/profile';

it('keeps readable Twitch colors and lightens dark colors without changing their hue', () => {
  expect(readableNameColor('#00FF00')).toBe('#00ff00');
  expect(readableNameColor('#ffffff')).toBe('#ffffff');
  const green = readableNameColor('#008000')!;
  const rgb = [1, 3, 5].map((offset) => parseInt(green.slice(offset, offset + 2), 16));
  expect(rgb[0]).toBe(rgb[2]);
  expect(rgb[1]).toBeGreaterThan(128);
  expect(rgb[1]).toBeGreaterThan(rgb[0]!);
  for (const color of [null, '', 'red', '#123', 'url(evil)', '#gggggg'])
    expect(readableNameColor(color)).toBeUndefined();
});
