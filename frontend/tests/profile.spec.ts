import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };
import type { AccountProfile } from '../src/shared/lib/types';

const headers = { 'X-TDM-Request': '1' };
const image = 'https://static-cdn.jtvnw.net/profile-fixture.png';
const profile: AccountProfile = {
  login: 'northwind',
  display_name: 'Northwind',
  avatar_url: image,
  banner_url: image,
  color: '#008000',
  description: 'Games, drops and good company.',
  created_at: '2023-04-09T16:03:17Z',
  followers: 13,
  roles: [],
  socials: [{ name: 'Website', url: 'https://example.org' }],
  badges: [
    {
      id: 'event',
      title: 'Event badge',
      description: 'Earned during a Twitch event.',
      image_url: image,
    },
  ],
  available_badges: Array.from({ length: 26 }, (_, i) => ({
    id: i === 0 ? 'event' : `badge-${i}`,
    title: i === 0 ? 'Event badge' : `Badge ${i}`,
    description: 'Earned during a Twitch event.',
    image_url: image,
  })),
};

test.beforeEach(async ({ page, request }) => {
  await request.post('/__test/reset', { headers, data: {} });
  await page.route('https://static-cdn.jtvnw.net/**', (route) =>
    route.fulfill({
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80"><rect width="80" height="80" fill="#56516b"/><circle cx="40" cy="40" r="22" fill="#d5cde9"/></svg>',
    }),
  );
  await request.post('/__test/event', {
    headers,
    data: { event: 'login_status', data: { ...fixture.login, profile } },
  });
});

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 1024, height: 360 },
  { width: 390, height: 844 },
  { width: 320, height: 568 },
]) {
  test(`account card shows global badges and stays reachable at ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await page.goto('/settings#account');
    const settings = page.locator('#account');
    const identity = settings.getByRole('button', { name: 'View Northwind profile' });
    await expect(identity).toBeVisible();
    await expect(identity.locator('img')).toHaveCount(1);
    await expect(settings.getByText('Twitch ID: 123456')).toHaveCount(0);
    const trigger =
      viewport.width >= 1024
        ? page.locator('aside').getByRole('button', { name: 'View Northwind profile' })
        : identity;
    if (viewport.width >= 1024) await expect(trigger.locator('img')).toHaveCount(2);
    await trigger.focus();
    await trigger.press('Enter');
    const card = page.getByRole('dialog', { name: 'Northwind', exact: true });
    await expect(card).toBeVisible();
    await expect(card).toContainText('Games, drops and good company.');
    await expect(card).toContainText('Apr 9, 2023');
    const badges = card.getByRole('region', { name: 'Global badges' });
    await expect(badges.getByRole('button')).toHaveCount(26);
    await badges.getByRole('button', { name: 'Event badge (equipped)', exact: true }).click();
    await expect(badges.getByRole('status')).toContainText('Earned during a Twitch event.');
    for (const button of await badges.getByRole('button').all())
      expect(await button.evaluate((node) => getComputedStyle(node).borderWidth)).toBe('0px');
    const box = (await card.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(viewport.width);
    expect(box.y + box.height).toBeLessThanOrEqual(viewport.height);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await card.screenshot({ path: `../artifacts/account-profile-${viewport.width}.png` });
    await page.keyboard.press('Escape');
    await expect(card).toBeHidden();
    await expect(trigger).toBeFocused();
    await trigger.click();
    await page.mouse.click(viewport.width - 5, 5);
    await expect(card).toBeHidden();
    await trigger.click();
    await card.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(card).toBeHidden();
  });
}

test('profile handles unavailable collections, broken art, account changes and logout', async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.route('https://static-cdn.jtvnw.net/**', (route) => route.abort());
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'login_status',
      data: { ...fixture.login, profile: { ...profile, available_badges: null } },
    },
  });
  await page.goto('/settings#account');
  const trigger = page.locator('aside').getByRole('button', { name: 'View Northwind profile' });
  await trigger.click();
  const card = page.getByRole('dialog', { name: 'Northwind', exact: true });
  await expect(card).toContainText('Full badge collection unavailable.');
  await expect(card.getByRole('region', { name: 'Global badges' }).getByRole('button')).toHaveCount(
    1,
  );
  await expect(card.locator('img')).toHaveCount(0);
  await request.post('/__test/event', {
    headers,
    data: { event: 'login_status', data: { status: 'Logged in', user_id: 99 } },
  });
  await expect(card).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'View Northwind profile' })).toHaveCount(0);
  await page.locator('aside').getByRole('button', { name: 'View Twitch account profile' }).click();
  await expect(page.getByRole('dialog')).toContainText(
    'Profile details are currently unavailable.',
  );
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Log out of Twitch', exact: true }).click();
  await expect(page.locator('.account-trigger')).toHaveCount(0);
});
