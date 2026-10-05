// Preserve Twitch's hue, lightening only colors that fail normal-text contrast
// against the darkest-to-lightest surfaces used by the profile controls (#333).
export function readableNameColor(color: string | null): string | undefined {
  if (!color || !/^#[\da-f]{6}$/i.test(color)) return undefined;
  const rgb = [1, 3, 5].map((offset) => parseInt(color.slice(offset, offset + 2), 16));
  const linear = (value: number) => {
    const channel = value / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  };
  for (let step = 0; step <= 100; step++) {
    const adjusted = rgb.map((channel) => Math.round(channel + ((255 - channel) * step) / 100));
    const luminance = adjusted.reduce(
      (total, channel, index) => total + linear(channel) * ([0.2126, 0.7152, 0.0722][index] ?? 0),
      0,
    );
    if ((luminance + 0.05) / (linear(51) + 0.05) >= 4.5)
      return `#${adjusted.map((channel) => channel.toString(16).padStart(2, '0')).join('')}`;
  }
  return undefined;
}
