import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

const headers = { 'X-TDM-Request': '1' };
test.beforeEach(async ({ request }) => {
  expect((await request.post('/__test/reset', { headers, data: {} })).ok()).toBeTruthy();
});

for (const width of [1440, 390, 320]) {
  test(`pause control preserves progress and fits the header at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await expect(page).toHaveTitle('70% Rust - Drops Miner');
    const card = page.locator('section[aria-labelledby="mining-heading"]');
    const bar = card.getByRole('progressbar');
    const progress = await bar.getAttribute('aria-valuenow');
    const pause = card.getByRole('button', { name: 'Pause mining', exact: true });
    await expect(pause).toBeEnabled();
    const bounds = (await pause.boundingBox())!;
    const cardBounds = (await card.boundingBox())!;
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(cardBounds.x + cardBounds.width);
    await pause.click();
    await expect(card.getByRole('heading', { name: 'Paused', exact: true })).toBeVisible();
    await expect(bar).toHaveAttribute('aria-valuenow', progress!);
    await expect(card.getByText('42 / 60 min', { exact: true })).toBeVisible();
    await expect(page).toHaveTitle('Paused - Drops Miner');
    await page.screenshot({ path: `../artifacts/mining-paused-${width}.png`, fullPage: true });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.reload();
    await expect(card.getByRole('heading', { name: 'Paused', exact: true })).toBeVisible();
    await card.getByRole('button', { name: 'Resume mining', exact: true }).click();
    await expect(card.getByRole('heading', { name: 'Now mining', exact: true })).toBeVisible();
    await page.getByRole('link', { name: 'Settings', exact: true }).click();
    await expect(page).toHaveTitle('70% Rust - Drops Miner');
  });
}

test('failed pause saves keep the running state visible and can be retried', async ({ page }) => {
  await page.goto('/');
  await page.route('**/api/settings', async (route) => {
    if (route.request().method() === 'POST')
      await route.fulfill({ status: 500, json: { error: 'request_failed' } });
    else await route.continue();
  });
  await page.getByRole('button', { name: 'Pause mining', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Try again', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Now mining', exact: true })).toBeVisible();
  await expect(page).toHaveTitle('70% Rust - Drops Miner');
  await page.unroute('**/api/settings');
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Paused', exact: true })).toBeVisible();
});
