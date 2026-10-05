import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };
import type { AccountProfile } from '../src/shared/lib/types';

const headers = { 'X-TDM-Request': '1' };
const image = 'https://static-cdn.jtvnw.net/profile-fixture.png';
const profile: AccountProfile = {
  display_name: 'Northwind',
  avatar_url: image,
  color: '#008000',
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
  test(`account settings show identity and borderless badges at ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    if (viewport.width >= 1024) {
      await page.goto('/');
      const link = page.locator('aside').getByRole('link', { name: 'Twitch account: Northwind' });
      await expect(link).toBeVisible();
      const avatar = link.locator('img').first();
      const badge = link.getByRole('img', { name: 'Event badge', exact: true });
      const name = link.getByText('Northwind', { exact: true });
      const avatarBox = (await avatar.boundingBox())!;
      const github = (await page
        .getByRole('link', { name: 'GitHub repository' })
        .locator('svg')
        .boundingBox())!;
      expect(avatarBox.width).toBe(32);
      expect(avatarBox.height).toBe(32);
      expect(avatarBox.width).toBe(github.width);
      expect(avatarBox.x).toBe(github.x);
      expect((await badge.boundingBox())!.x).toBeGreaterThanOrEqual(avatarBox.x + avatarBox.width);
      expect((await name.boundingBox())!.x).toBeGreaterThan((await badge.boundingBox())!.x);
      await link.focus();
      await link.press('Enter');
    } else {
      await page.goto('/settings#account');
    }
    await expect(page).toHaveURL(/\/settings#account$/);
    const settings = page.locator('#account');
    const identity = settings.locator('.account-identity');
    await expect(identity.getByText('Northwind', { exact: true })).toBeVisible();
    await expect(identity.getByRole('img', { name: 'Event badge', exact: true })).toBeVisible();
    await expect(page.locator('.account-card')).toHaveCount(0);
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(settings).not.toContainText('Global badges');
    await expect(settings.locator('time, dl, a, [popover]')).toHaveCount(0);
    const badges = settings.getByRole('region', { name: 'Badges', exact: true });
    await expect(badges.getByRole('button')).toHaveCount(26);
    const equipped = badges.getByRole('button', { name: 'Event badge (equipped)', exact: true });
    await equipped.focus();
    await equipped.press('Enter');
    await expect(badges.getByRole('status')).toContainText('Earned during a Twitch event.');
    for (const button of await badges.getByRole('button').all()) {
      expect(await button.evaluate((node) => getComputedStyle(node).borderWidth)).toBe('0px');
      if (viewport.width < 768)
        expect((await button.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    }
    await badges.getByRole('button').last().scrollIntoViewIfNeeded();
    await expect(badges.getByRole('button').last()).toBeInViewport();
    await expect(
      page.getByRole('navigation', { name: 'Main navigation', exact: true }),
    ).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(viewport.width);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await settings.screenshot({ path: `../artifacts/account-settings-${viewport.width}.png` });
  });
}

test('account settings handle missing and empty badge collections, broken art, account changes and logout', async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.route('https://static-cdn.jtvnw.net/**', (route) => route.abort());
  const publish = async (value: unknown) =>
    request.post('/__test/event', {
      headers,
      data: { event: 'login_status', data: value },
    });
  await publish({ ...fixture.login, profile: { ...profile, available_badges: null } });
  await page.goto('/settings#account');
  const settings = page.locator('#account');
  const badges = settings.getByRole('region', { name: 'Badges', exact: true });
  await expect(settings).toContainText('Full badge collection unavailable.');
  await expect(badges.getByRole('button')).toHaveCount(1);
  await expect(settings.locator('img')).toHaveCount(0);
  await badges.getByRole('button').click();
  await expect(badges.getByRole('status')).toContainText('Event badge');
  await publish({ status: 'Logged in', user_id: 99 });
  await expect(settings.getByText('Northwind')).toHaveCount(0);
  await expect(badges.getByRole('status')).toHaveCount(0);
  await expect(settings).toContainText('Profile details are currently unavailable.');
  await publish({
    status: 'Logged in',
    user_id: 99,
    profile: { ...profile, display_name: 'Other account', badges: [], available_badges: [] },
  });
  await expect(settings).toContainText('No badges available.');
  await expect(settings).not.toContainText('Full badge collection unavailable.');
  await page.getByRole('button', { name: 'Log out of Twitch', exact: true }).click();
  await expect(settings.locator('.account-identity')).toHaveCount(0);
  await expect(badges).toHaveCount(0);
  await expect(page.locator('.account-trigger')).toHaveCount(0);
});
