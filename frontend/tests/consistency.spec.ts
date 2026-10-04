import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { mdiDockRight } from '@mdi/js';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
const activity = Array.from({ length: 80 }, (_, id) => ({
  id,
  first_at: '2026-10-03T12:00:00Z',
  last_at: '2026-10-03T12:00:00Z',
  category: 'claims',
  severity: 'info',
  code: 'claimed',
  args: {},
  message: `Recorded reward ${id}`,
  campaign_id: 'campaign-1',
  drop_id: 'reward-1',
  channel_id: null,
  count: 1,
  recovered: false,
}));

test.beforeEach(async ({ request }) => {
  expect((await request.get('/__test/health')).ok()).toBeTruthy();
  expect(await (await request.post('/__test/reset', { headers, data: {} })).json()).toEqual({
    ok: true,
  });
});

test('Activity fits short desktop and phone viewports and distinguishes empty filters', async ({
  page,
  request,
}) => {
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, activity } },
  });
  for (const viewport of [
    { width: 1440, height: 500 },
    { width: 390, height: 400 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/activity');
    const log = page.locator('#activity-list');
    await expect(log.locator('article')).toHaveCount(80);
    expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBeLessThanOrEqual(
      viewport.height,
    );
    expect(await log.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
    await expect(page.getByRole('heading', { name: 'Activity' })).toBeInViewport();
    await expect(page.getByRole('searchbox')).toBeInViewport();
    await page.getByRole('searchbox').fill('no such event');
    await expect(page.getByText('No matching activity', { exact: true })).toBeVisible();
    await page.screenshot({
      path: `../artifacts/consistency-activity-${viewport.width}-${viewport.height}.png`,
    });
  }
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, console: [], activity: [] } },
  });
  await expect(page.getByText('No activity yet', { exact: true })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test('Settings tabs keep their position, drafts and browser history', async ({ page }) => {
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 700 });
    await page.goto('/settings#account');
    const tabs = page.getByRole('navigation', { name: 'Settings sections' });
    const top = (await tabs.boundingBox())!.y;
    await tabs.getByRole('link', { name: 'Dashboard access' }).click();
    await page
      .getByLabel('New password (at least 8 characters)', { exact: true })
      .fill('unsaved-draft');
    for (const name of ['Connection', 'Maintenance', 'Twitch account', 'Dashboard access']) {
      await tabs.getByRole('link', { name, exact: true }).click();
      expect((await tabs.boundingBox())!.y).toBeCloseTo(top, 0);
      await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeInViewport();
      expect(await page.evaluate(() => scrollY)).toBe(0);
    }
    await expect(
      page.getByLabel('New password (at least 8 characters)', { exact: true }),
    ).toHaveValue('unsaved-draft');
    await page.goBack();
    await expect(tabs.getByRole('link', { name: 'Twitch account' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect((await tabs.boundingBox())!.y).toBeCloseTo(top, 0);
  }
});

test('Mining and Activity details return to the originating link and scroll position', async ({
  page,
  request,
}) => {
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, activity } },
  });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 700 });
    for (const id of [
      'mining-drop-details',
      'up-next-campaign-campaign-1',
      'up-next-drop-campaign-1-reward-2',
      'activity-campaign-30',
    ]) {
      const path = id.startsWith('activity') ? '/activity' : '/';
      await page.goto(path);
      const link = page.locator(`#${id}`);
      await link.evaluate((element) => element.scrollIntoView({ block: 'center' }));
      const top = await page.evaluate(() => scrollY);
      const lists = await page
        .locator('[data-restore-scroll]')
        .evaluateAll((elements) => elements.map((el) => el.scrollTop));
      if (path === '/activity')
        await expect(link.locator('path')).toHaveAttribute('d', mdiDockRight);
      await link.click();
      await expect(page.getByRole('complementary', { name: 'Campaign details' })).toBeVisible();
      await page.getByRole('button', { name: 'Close details' }).click();
      await expect(page).toHaveURL(new RegExp(path === '/' ? '/$' : '/activity$'));
      await expect(link).toBeFocused();
      await expect.poll(() => page.evaluate(() => scrollY)).toBeCloseTo(top, 0);
      expect(
        await page
          .locator('[data-restore-scroll]')
          .evaluateAll((elements) => elements.map((el) => el.scrollTop)),
      ).toEqual(lists);
      await link.press('Enter');
      await page.goBack();
      await expect(link).toBeFocused();
    }
  }
  await page.goto('/campaigns?campaign=campaign-1&drop=reward-1');
  await page.getByRole('button', { name: 'Close details' }).click();
  await expect(page).toHaveURL(/\/campaigns$/);
});

test('Available rewards show claims once with truthful times and uncropped artwork', async ({
  page,
  request,
}) => {
  const image = 'https://example.test/reward.png';
  await page.route('https://example.test/**', (route) =>
    route.fulfill({
      contentType: 'image/png',
      body: Buffer.from(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aVRsAAAAASUVORK5CYII=',
        'base64',
      ),
    }),
  );
  const campaign = structuredClone(fixture.campaigns[0]!);
  campaign.claimed_drops = 1;
  campaign.drops[0]!.is_claimed = true;
  campaign.drops[0]!.benefits.push({
    name: 'Additional benefit',
    type: 'DIRECT_ENTITLEMENT',
    image_url: image,
  });
  campaign.drops.forEach((drop) => {
    drop.benefits[0]!.image_url = image;
  });
  const entries = campaign.drops.map((drop) => ({
    id: drop.id,
    campaign_id: campaign.id,
    campaign: campaign.name,
    game: campaign.game_name,
    drop_name: drop.name,
    benefits: [drop.name],
    required_minutes: drop.required_minutes,
    claimed_at: '2026-10-03T06:24:00Z',
    claimed_at_is_observed: false,
    image_url: image,
  }));
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, campaigns: [campaign] } },
  });
  await page.route('**/api/history', async (route) =>
    route.fulfill({ json: { ...(await (await route.fetch()).json()), entries } }),
  );
  await page.goto('/');
  await expect(page.locator('[aria-labelledby="mining-heading"] img')).toHaveCSS(
    'object-fit',
    'contain',
  );
  await page.locator('#mining-drop-details').click();
  const detail = page.getByRole('complementary', { name: 'Campaign details' });
  const claimed = page.locator('#drop-reward-1');
  await expect(claimed.getByText(/^Claimed .+2026/)).toBeVisible();
  await expect(claimed.getByText('Explorer jacket', { exact: true })).toHaveCount(1);
  await expect(claimed.getByText('Additional benefit', { exact: true })).toBeVisible();
  await expect(claimed).toHaveCSS('border-left-width', '0px');
  await expect(claimed).toHaveCSS('padding-left', '0px');
  await expect(claimed).toHaveAttribute('aria-current', 'true');
  await expect(detail.getByRole('heading', { name: 'History', exact: true })).toHaveCount(0);
  await expect(page.locator('#drop-reward-2').getByText(/^Claimed/)).toHaveCount(0);
  for (const art of await detail.locator('.reward-detail img').all())
    await expect(art).toHaveCSS('object-fit', 'contain');
  entries[0]!.claimed_at_is_observed = true;
  await page.reload();
  await expect(claimed.getByText(/^Claimed · First observed/)).toBeVisible();
  await page.goto('/campaigns?tab=history&campaign=campaign-1&drop=reward-1');
  await expect(page.locator('#history-drop-reward-1 img')).toHaveCSS('object-fit', 'contain');
  await expect(page.locator('#history-drop-reward-1')).toHaveCSS('border-left-width', '0px');
  await expect(detail.getByRole('progressbar')).toHaveCount(0);
});

test('Up next shares identical windows but retains distinct dates and upcoming labels', async ({
  page,
  request,
}) => {
  const state = structuredClone(fixture);
  const drops = state.wanted_items[0]!.campaigns[0]!.drops;
  drops[1]!.starts_at = '2026-09-20T02:00:00+02:00';
  drops[1]!.ends_at = '2026-10-02T02:00:00+02:00';
  await request.post('/__test/event', { headers, data: { event: 'initial_state', data: state } });
  await page.goto('/');
  const queue = page.locator('#up-next-list');
  await expect(queue.getByText(/^Ends:/)).toHaveCount(1);
  await expect(queue.locator('li').getByText(/^Ends:/)).toHaveCount(0);
  drops[1]!.ends_at = '2026-10-03T00:00:00Z';
  await request.post('/__test/event', { headers, data: { event: 'initial_state', data: state } });
  await expect(queue.locator('li').getByText(/^Ends:/)).toHaveCount(2);
  drops[1]!.eligibility = 'upcoming';
  await request.post('/__test/event', { headers, data: { event: 'initial_state', data: state } });
  await expect(queue.locator('li').getByText(/^Starts:/)).toHaveCount(1);
  await expect(queue.locator('li').getByText(/^Ends:/)).toHaveCount(1);
});

test('Mining lists have modest gutters, inset separators and full phone touch targets', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  for (const id of ['channels-list', 'up-next-list']) {
    const list = page.locator(`#${id}`);
    await expect(list).toHaveCSS('padding-right', '4px');
    const first = list.locator(':scope > div').first();
    await expect(first).toHaveCSS('margin-left', '16px');
    await expect(first).toHaveCSS('margin-right', '16px');
  }
  await page.goto('/?edit=priorities');
  await expect(page.getByRole('region', { name: 'Game priorities' })).toHaveCSS(
    'padding-right',
    '4px',
  );
  await expect(page.locator('[data-game]').first()).toHaveCSS('margin-left', '16px');
  await page.setViewportSize({ width: 390, height: 844 });
  const grip = page.locator('.drag-handle').first();
  expect((await grip.boundingBox())!.width).toBe(44);
  expect((await grip.boundingBox())!.height).toBe(44);
  await grip.hover();
  await expect(grip).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  const check = page.getByRole('checkbox', { name: 'Badge', exact: true }).locator('..');
  expect((await check.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await page.getByRole('searchbox', { name: 'Search games...' }).fill('Elder');
  const result = page.getByRole('region', { name: 'Search games...' }).getByRole('button').first();
  expect((await result.boundingBox())!.height).toBeGreaterThanOrEqual(44);
});

test('Campaigns keeps refresh beside tabs and gives search the full phone width', async ({
  page,
}) => {
  for (const width of [320, 390, 1440]) {
    await page.setViewportSize({ width, height: 844 });
    for (const tab of ['', '?tab=history']) {
      await page.goto(`/campaigns${tab}`);
      const tabs = (await page.getByRole('navigation', { name: 'Campaign views' }).boundingBox())!;
      const refresh = (await page
        .getByRole('button', { name: 'Refresh inventory', exact: true })
        .boundingBox())!;
      expect(Math.abs(tabs.y + tabs.height / 2 - refresh.y - refresh.height / 2)).toBeLessThan(1);
      const search = (await page.getByRole('searchbox').boundingBox())!;
      if (width < 768) expect(search.width).toBe(width - 32);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      await page.screenshot({
        path: `../artifacts/consistency-campaigns-${width}-${tab ? 'history' : 'available'}.png`,
      });
    }
  }
  await page.setViewportSize({ width: 390, height: 844 });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
