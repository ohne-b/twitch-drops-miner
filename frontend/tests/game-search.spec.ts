import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
const stardew = {
  id: '490744',
  name: 'Stardew Valley',
  box_art_url: 'https://static-cdn.jtvnw.net/ttv-boxart/490744-{width}x{height}.jpg',
};
test.beforeEach(async ({ request, page }) => {
  expect((await request.post('/__test/reset', { headers, data: {} })).ok()).toBeTruthy();
  await page.route('https://static-cdn.jtvnw.net/**', (route) =>
    route.fulfill({
      contentType: 'image/png',
      body: Buffer.from(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lVEAAAAASUVORK5CYII=',
        'base64',
      ),
    }),
  );
});

test('search adds a game without campaigns and persists its official name and cover', async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route('**/api/games', (route) =>
    route.fulfill({ json: route.request().postDataJSON().search ? [stardew] : [] }),
  );
  await page.goto('/?edit=priorities');
  const search = page.getByRole('searchbox', { name: 'Search games' });
  await search.fill('stardew');
  const result = page.getByRole('button', { name: 'Stardew Valley', exact: true });
  await expect(result).toBeVisible();
  await expect(result.locator('img')).toHaveAttribute('src', /490744-80x112.jpg/);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await result.click();
  const row = page.locator('[data-game="Stardew Valley"]');
  await expect(row).toBeVisible();
  await expect(row.locator('img')).toHaveAttribute('src', /490744-80x112.jpg/);
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).game_metadata)
    .toEqual([stardew]);
  await page.reload();
  await expect(row.locator('img')).toHaveAttribute('src', /490744-80x112.jpg/);
  await page.screenshot({ path: '../artifacts/game-search-mobile.png', fullPage: true });
  await row.getByRole('button', { name: 'Remove Stardew Valley' }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).game_metadata)
    .toEqual([]);
});

test('old Unicode saved names gain covers without a campaign and retain their priority', async ({
  page,
  request,
}) => {
  const state = structuredClone(fixture);
  state.settings.games_to_watch = ['Rust', 'STRASSE'];
  const metadata = { ...stardew, name: 'Straße' };
  expect(
    (
      await request.post('/__test/event', {
        headers,
        data: { event: 'initial_state', data: state },
      })
    ).ok(),
  ).toBeTruthy();
  await page.route('**/api/games', (route) => route.fulfill({ json: [metadata] }));
  await page.goto('/?edit=priorities');
  await expect(page.locator('[data-game="STRASSE"] img')).toHaveAttribute(
    'src',
    /490744-80x112.jpg/,
  );
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).game_metadata)
    .toEqual([metadata]);
  expect((await (await request.get('/api/settings')).json()).games_to_watch).toEqual([
    'Rust',
    'STRASSE',
  ]);
});

test('changed queries ignore stale results and failed searches can retry or add a name', async ({
  page,
}) => {
  let finishSlow: (() => void) | undefined;
  await page.route('**/api/games', async (route) => {
    const query = route.request().postDataJSON();
    if (query.names) return route.fulfill({ json: [] });
    if (query.search === 'slow') {
      await new Promise<void>((resolve) => {
        finishSlow = resolve;
      });
      return route.fulfill({ json: [{ ...stardew, name: 'Old result' }] });
    }
    if (query.search === 'fail')
      return route.fulfill({ status: 503, json: { detail: 'request_failed' } });
    return route.fulfill({ json: [stardew] });
  });
  await page.goto('/?edit=priorities');
  const search = page.getByRole('searchbox', { name: 'Search games' });
  await search.fill('slow');
  await expect.poll(() => !!finishSlow).toBe(true);
  await expect(page.getByRole('region', { name: 'Search games', exact: true })).toHaveCount(0);
  await expect(page.getByText('Searching Twitch…', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Add Game', exact: true })).toBeDisabled();
  await search.fill('stardew');
  await expect(page.getByRole('button', { name: 'Stardew Valley', exact: true })).toBeVisible();
  finishSlow!();
  await expect(page.getByRole('button', { name: 'Old result', exact: true })).toHaveCount(0);
  await search.fill('fail');
  await expect(
    page.getByText('Twitch search is unavailable. You can still add a name.'),
  ).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add Game', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Add Game', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(page.locator('[data-game="fail"]')).toBeVisible();
  await search.fill('fail');
  await expect(
    page.getByText('Twitch search is unavailable. You can still add a name.'),
  ).toBeVisible();
  await page.unroute('**/api/games');
  await page.route('**/api/games', (route) => route.fulfill({ json: [stardew] }));
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Stardew Valley', exact: true })).toBeVisible();
});

test('early campaign selections get covers and Enter accepts Twitch word matches', async ({
  page,
  request,
}) => {
  let releaseSearch: (() => void) | undefined;
  const elder = { ...stardew, id: '65654', name: 'The Elder Scrolls Online' };
  const warcraft = { ...stardew, id: '18122', name: 'World of Warcraft' };
  await page.route('**/api/games', async (route) => {
    const query = route.request().postDataJSON();
    if (query.names)
      return route.fulfill({ json: query.names.includes(elder.name) ? [elder] : [] });
    if (query.search === 'elder') {
      await new Promise<void>((resolve) => {
        releaseSearch = resolve;
      });
      return route.fulfill({ json: [elder] });
    }
    return route.fulfill({ json: [warcraft] });
  });
  await page.goto('/?edit=priorities');
  const search = page.getByRole('searchbox', { name: 'Search games' });
  await search.fill('elder');
  await expect.poll(() => !!releaseSearch).toBe(true);
  await expect(page.getByText('Searching Twitch…', { exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: elder.name, exact: true }).click();
  releaseSearch!();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).game_metadata)
    .toEqual([elder]);
  await search.fill('world warcraft');
  await expect(page.getByRole('button', { name: warcraft.name, exact: true })).toBeVisible();
  await search.press('Enter');
  await expect(page.locator('[data-game="World of Warcraft"]')).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});
