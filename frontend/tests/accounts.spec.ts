import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };
import type { AccountsOverview } from '../src/app/accounts';

const headers = { 'X-TDM-Request': '1' };
const second = '00000000000000000000000000000001';
const third = '00000000000000000000000000000002';
const overview = async (request: APIRequestContext): Promise<AccountsOverview> =>
  (await request.get('/api/accounts')).json();
const settings = async (request: APIRequestContext, id: string) =>
  (await request.get(`/api/accounts/${id}/settings`)).json();
async function switchTo(page: Page, name: string) {
  await page
    .getByRole('button', { name: 'Switch account', exact: true })
    .filter({ visible: true })
    .click();
  await page
    .getByRole('group', { name: 'Switch account', exact: true })
    .getByRole('button', { name: new RegExp(`^${name}`) })
    .click();
}
async function seed(request: APIRequestContext, count = 2) {
  const snapshots = Array.from({ length: count }, (_, index) => ({
    ...fixture,
    login: {
      status: 'Logged in',
      user_id: 200 + index,
      profile: {
        display_name: `Account ${index + 2}`,
        login: `account${index + 2}`,
        avatar_url: null,
        color: null,
        badges: [],
        available_badges: [],
      },
    },
    settings: {
      ...fixture.settings,
      games_to_watch: index % 2 ? ['Other'] : ['Rust'],
      mining_paused: index % 2 === 1,
      auto_claim_drops: false,
    },
    current_drop: fixture.current_drop
      ? { ...fixture.current_drop, drop_name: `Reward ${index + 2}` }
      : null,
  }));
  expect((await request.post('/__test/accounts', { headers, data: snapshots })).ok()).toBe(true);
}
test.beforeEach(async ({ request, page }) => {
  await request.post('/__test/reset', { headers, data: {} });
  page.on('pageerror', (error) => {
    throw error;
  });
});

for (const width of [1440, 390, 320]) {
  test(`account overview, bulk controls and switcher fit ${width}px`, async ({ page, request }) => {
    await seed(request, 19);
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/settings#account');
    await expect(
      page.getByRole('checkbox', { name: 'Select Account 20', exact: true }),
    ).toBeAttached();
    await expect(page.getByRole('button', { name: 'Add account', exact: true })).toBeDisabled();
    await page.getByRole('checkbox', { name: 'Select all', exact: true }).check();
    const toolbar = page.getByRole('button', { name: 'Add account', exact: true }).locator('..');
    const buttons = toolbar.getByRole('button');
    await expect(buttons).toHaveCount(4);
    const boxes = await buttons.evaluateAll((elements) =>
      elements.map((element) => ({
        text: element.textContent,
        y: element.getBoundingClientRect().y,
      })),
    );
    expect(boxes.every((box) => box.text === '' && box.y === boxes[0]!.y)).toBe(true);
    await toolbar.screenshot({ path: `../artifacts/account-toolbar-${width}.png` });
    await page.getByRole('button', { name: 'Pause', exact: true }).click();
    await expect
      .poll(async () => (await overview(request)).accounts.every((account) => account.paused))
      .toBe(true);
    await expect(page.locator('#settings-account').getByRole('status')).toHaveCount(0);
    await page.getByRole('button', { name: 'Resume', exact: true }).click();
    await expect
      .poll(async () => (await overview(request)).accounts.every((account) => !account.paused))
      .toBe(true);
    await page
      .getByRole('button', { name: 'Switch account', exact: true })
      .filter({ visible: true })
      .click();
    const picker = page.getByRole('group', { name: 'Switch account', exact: true });
    await picker.getByRole('searchbox').fill('Account 20');
    await expect(picker.getByRole('button')).toHaveCount(2); // Result and clear-search control.
    await picker.getByRole('button', { name: /^Account 20/ }).click();
    await expect(page).toHaveURL(/account=00000000000000000000000000000013/);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({ path: `../artifacts/accounts-${width}.png` });
  });
}

for (const width of [1440, 320]) {
  test(`one account has direct settings navigation and no table at ${width}px`, async ({
    page,
    request,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await page
      .getByRole('link', { name: 'Manage accounts', exact: true })
      .filter({ visible: true })
      .click();
    await expect(page).toHaveURL(/\/settings#account$/);
    await expect(page.getByRole('button', { name: 'Switch account', exact: true })).toHaveCount(0);
    await expect(page.getByRole('checkbox', { name: 'Select all', exact: true })).toHaveCount(0);
    await expect(page.locator('#settings-account [aria-label="Accounts"]')).toHaveCount(0);
    for (const name of [
      'Add account',
      'Rename Twitch 123456',
      'Sign out Twitch 123456',
      'Remove Twitch 123456',
    ])
      await expect(page.getByRole('button', { name, exact: true })).toBeEnabled();
    await page.getByRole('button', { name: 'Rename Twitch 123456', exact: true }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await page.screenshot({ path: `../artifacts/single-account-${width}.png`, fullPage: true });
    await seed(request, 1);
    await expect(page.getByRole('checkbox', { name: 'Select all', exact: true })).toBeVisible();
    await expect(
      page.getByRole('button', { name: 'Switch account', exact: true }).filter({ visible: true }),
    ).toBeVisible();
    await page.getByRole('checkbox', { name: 'Select all', exact: true }).check();
    await page.getByRole('button', { name: 'Pause', exact: true }).click();
    await expect
      .poll(async () => (await overview(request)).accounts.every((account) => account.paused))
      .toBe(true);
    await expect(page.locator('#settings-account').getByRole('status')).toHaveCount(0);
    await page.getByRole('button', { name: 'Remove Account 2', exact: true }).click();
    await page
      .getByRole('dialog')
      .getByRole('button', { name: 'Remove account', exact: true })
      .click();
    await expect(page.getByRole('checkbox', { name: 'Select all', exact: true })).toHaveCount(0);
    await expect(page.locator('#settings-account').getByRole('status')).toHaveCount(0);
    await expect(
      page.getByRole('link', { name: 'Manage accounts', exact: true }).filter({ visible: true }),
    ).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
}

test('the single-account shortcut recovers an unavailable account link', async ({ page }) => {
  await page.goto('/?account=missing');
  const manageLinks = page
    .getByRole('link', { name: 'Manage accounts', exact: true })
    .filter({ visible: true });
  for (const link of await manageLinks.all())
    await expect(link).toHaveAttribute('href', '/settings?account=primary#account');
  const manage = page
    .locator('aside')
    .getByRole('link', { name: 'Manage accounts', exact: true })
    .filter({ visible: true });
  await expect(manage).toHaveAttribute('href', '/settings?account=primary#account');
  await manage.click();
  await expect(page).toHaveURL(/\/settings\?account=primary#account$/);
  await expect(page.locator('#settings-account .account-identity')).toContainText('Twitch 123456');
  await expect(
    page.getByRole('button', { name: 'Sign out Twitch 123456', exact: true }),
  ).toBeEnabled();
});

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 1280, height: 360 },
  { width: 320, height: 640 },
]) {
  test(`account header and login help fit ${viewport.width}x${viewport.height}`, async ({
    page,
    request,
  }) => {
    await seed(request);
    await page.setViewportSize(viewport);
    await page.goto('/settings#account');
    const add = page.getByRole('button', { name: 'Add account', exact: true });
    const header = add.locator('..');
    await expect(header.getByRole('checkbox', { name: 'Select all', exact: true })).toBeVisible();
    const addBox = (await add.boundingBox())!;
    const headerBox = (await header.boundingBox())!;
    expect(headerBox.x + headerBox.width - addBox.x - addBox.width).toBeCloseTo(12);
    await expect(page.locator('#settings-account .account-identity').locator('../..')).toHaveCSS(
      'border-top-width',
      '0px',
    );
    await add.click();
    const dialog = page.getByRole('dialog', { name: 'Add account', exact: true });
    const heading = dialog.getByRole('heading', { name: 'Add account', exact: true });
    const help = dialog.getByRole('button', { name: 'Add account help', exact: true });
    await expect(help).toBeVisible();
    const headingBox = (await heading.boundingBox())!;
    const helpBox = (await help.boundingBox())!;
    expect(helpBox.x).toBeGreaterThanOrEqual(headingBox.x + headingBox.width);
    expect(
      Math.abs(helpBox.y + helpBox.height / 2 - headingBox.y - headingBox.height / 2),
    ).toBeLessThan(1);
    await help.click();
    const note = dialog.getByRole('note');
    await expect(note).toContainText(
      'Check which account is signed in on Twitch before entering the code.',
    );
    const bounds = await note.evaluate((element) => ({
      client: element.clientHeight,
      scroll: element.scrollHeight,
      top: element.getBoundingClientRect().top,
      bottom: element.getBoundingClientRect().bottom,
    }));
    expect(bounds.scroll).toBeLessThanOrEqual(bounds.client + 1);
    expect(bounds.top).toBeGreaterThanOrEqual(0);
    expect(bounds.bottom).toBeLessThanOrEqual(viewport.height);
    if (viewport.width >= 768)
      expect(
        Math.min(
          Math.abs(bounds.top - helpBox.y - helpBox.height),
          Math.abs(helpBox.y - bounds.bottom),
        ),
      ).toBeCloseTo(8);
    await page.screenshot({ path: `../artifacts/account-login-help-${viewport.width}.png` });
    await page.keyboard.press('Escape');
    await expect(note).not.toBeVisible();
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Done', exact: true })).toBeEnabled();
    await dialog.getByRole('button', { name: 'Done', exact: true }).click();
    for (const label of ['Display label', 'Mining preferences']) {
      const fieldHelp = dialog.getByRole('button', { name: `${label} help`, exact: true });
      await fieldHelp.click();
      const fieldNote = dialog.getByRole('note', { name: `${label} help`, exact: true });
      await expect(fieldNote).toBeVisible();
      const box = (await fieldNote.boundingBox())!;
      const anchor = (await fieldHelp.boundingBox())!;
      expect(
        await fieldNote.evaluate((element) => element.scrollHeight <= element.clientHeight + 1),
      ).toBe(true);
      expect(box.y).toBeGreaterThanOrEqual(0);
      expect(box.y + box.height).toBeLessThanOrEqual(viewport.height);
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(viewport.width);
      if (viewport.width >= 768)
        expect(
          Math.min(
            Math.abs(box.y - anchor.y - anchor.height),
            Math.abs(anchor.y - box.y - box.height),
          ),
        ).toBeCloseTo(8);
      await page.screenshot({
        path: `../artifacts/account-${label.replaceAll(' ', '-')}-help-${viewport.width}.png`,
      });
      await page.keyboard.press('Escape');
      await expect(fieldNote).not.toBeVisible();
      await expect(dialog).toBeVisible();
    }
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
}

test('switching retains account views and sends a pending preference save to its original account', async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto('/?edit=priorities');
  const automatic = page.getByRole('checkbox', { name: 'Automatically claim drops', exact: true });
  await automatic.uncheck();
  await switchTo(page, 'Account 2');
  await expect(automatic).not.toBeChecked();
  await automatic.check();
  await switchTo(page, 'Twitch 123456');
  await expect(automatic).not.toBeChecked();
  await expect.poll(async () => (await settings(request, 'primary')).auto_claim_drops).toBe(false);
  await expect.poll(async () => (await settings(request, second)).auto_claim_drops).toBe(true);
  expect((await settings(request, third)).auto_claim_drops).toBe(false);
  await page.getByRole('link', { name: 'Campaigns', exact: true }).click();
  await page
    .getByRole('searchbox', { name: 'Search campaigns and rewards', exact: true })
    .fill('Rust');
  await switchTo(page, 'Account 2');
  await expect(page).toHaveURL(new RegExp(`/campaigns\\?account=${second}$`));
  await switchTo(page, 'Twitch 123456');
  await expect(
    page.getByRole('searchbox', { name: 'Search campaigns and rewards', exact: true }),
  ).toHaveValue('Rust');
});

test('mobile account switching restores the content and campaign list scroll positions', async ({
  page,
  request,
}) => {
  await seed(request, 1);
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_batch_update',
      data: {
        campaigns: Array.from({ length: 25 }, (_, index) => ({
          ...fixture.campaigns[0],
          id: `scroll-${index}`,
        })),
      },
    },
  });
  await page.setViewportSize({ width: 390, height: 600 });
  for (const view of ['grid', 'list']) {
    await page.goto(`/campaigns?account=primary&view=${view}`);
    await expect(page.locator('.campaign-summary')).toHaveCount(25);
    const scroller = page.locator(view === 'grid' ? '#main' : '.campaign-results-list');
    await scroller.evaluate((element) => element.scrollTo(0, 400));
    await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBe(400);
    await switchTo(page, 'Account 2');
    await expect(page).toHaveURL(new RegExp(`account=${second}`));
    await switchTo(page, 'Twitch 123456');
    await expect(page.locator('.campaign-summary')).toHaveCount(25);
    await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBe(400);
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
  }
});

test('switching accounts keeps the current page and settings or campaign section', async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto('/settings#notifications');
  await switchTo(page, 'Account 2');
  await expect(page).toHaveURL(new RegExp(`/settings\\?account=${second}#notifications$`));
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await switchTo(page, 'Twitch 123456');
  await expect(page).toHaveURL(/\/\?account=primary$/);
  await page.getByRole('link', { name: 'Settings', exact: true }).click();
  await page.getByRole('link', { name: 'Connection', exact: true }).click();
  await switchTo(page, 'Account 2');
  await expect(page).toHaveURL(new RegExp(`/settings\\?account=${second}#connection$`));
  await page.goto(`/campaigns?account=${second}&tab=history`);
  await switchTo(page, 'Twitch 123456');
  const url = new URL(page.url());
  expect(url.pathname).toBe('/campaigns');
  expect(url.searchParams.get('tab')).toBe('history');
  expect(url.searchParams.get('account')).toBe('primary');
});

for (const width of [1440, 320]) {
  test(`Settings navigation retains the viewed account and preference return path at ${width}px`, async ({
    page,
    request,
  }) => {
    await seed(request);
    await page.setViewportSize({ width, height: 844 });
    await page.goto(`/settings?account=${second}`);
    const menu = page.getByRole('navigation', { name: 'Settings sections' });
    await menu.getByRole('link', { name: 'Connection', exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`/settings\\?account=${second}#connection$`));
    const proxy = page.getByLabel('Proxy URL', { exact: true });
    await proxy.fill('http://127.0.0.1:8888');
    await switchTo(page, 'Twitch 123456');
    await expect(page).toHaveURL(/\/settings\?account=primary#connection$/);
    await expect(proxy).toHaveValue('http://127.0.0.1:8888');
    await proxy.fill('');
    if (width < 1024) await page.getByRole('link', { name: 'Back to Settings' }).click();
    const mining = menu.getByRole('link', { name: 'Mining preferences', exact: true });
    await mining.click();
    await expect(page).toHaveURL(/\/\?edit=priorities&account=primary$/);
    await switchTo(page, 'Account 2');
    await expect(page).toHaveURL(new RegExp(`/\\?edit=priorities&account=${second}$`));
    await page.getByRole('link', { name: 'Back to Settings' }).click();
    await expect(mining).toBeFocused();
    expect(new URL(page.url()).searchParams.get('account')).toBe(second);
    await page.goto(`/settings?account=${second}#mining`);
    await expect(page).toHaveURL(new RegExp(`/\\?edit=priorities&account=${second}$`));
    await expect(page.getByRole('link', { name: 'Back to Settings' })).toBeVisible();
    await page
      .getByRole('navigation', { name: 'Main navigation' })
      .getByRole('link', { name: 'Mining', exact: true })
      .click();
    await page.getByRole('link', { name: 'Edit', exact: true }).click();
    await switchTo(page, 'Twitch 123456');
    await expect(page.getByRole('link', { name: 'Back to Mining', exact: true })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Back to Settings', exact: true })).toHaveCount(0);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
}

test('bulk actions keep successful accounts and retry only failures after a revision conflict', async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto('/settings#account');
  await page.getByRole('checkbox', { name: 'Select all', exact: true }).check();
  let conflict = true;
  const targets: string[][] = [];
  await page.route('**/api/accounts', async (route) => {
    if (route.request().method() !== 'POST') return route.continue();
    const action = route.request().postDataJSON();
    if (action.action === 'pause') {
      targets.push(action.targets.map((target: { id: string }) => target.id));
      if (conflict) {
        conflict = false;
        const before = await settings(request, second);
        await request.post(`/api/accounts/${second}/settings`, {
          headers,
          data: { revision: before.revision, games_to_watch: ['Changed elsewhere'] },
        });
      }
    }
    await route.continue();
  });
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Account 2');
  await page.getByRole('button', { name: 'Retry failed accounts', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Retry failed accounts', exact: true }),
  ).toHaveCount(0);
  await expect(page.locator('#settings-account').getByRole('status')).toHaveCount(0);
  expect(targets).toEqual([['primary', second, third], [second]]);
  expect((await settings(request, second)).games_to_watch).toEqual(['Changed elsewhere']);
});

test('copying preferences preserves target pause and automatic claims unless explicitly included', async ({
  page,
  request,
}) => {
  await seed(request);
  const original = await settings(request, third);
  await page.goto('/settings#account');
  await page.getByRole('checkbox', { name: 'Select Account 3', exact: true }).check();
  await page.getByRole('button', { name: 'Copy mining preferences', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Copy mining preferences', exact: true });
  await dialog.getByRole('combobox').selectOption('primary');
  await expect(
    dialog.getByRole('checkbox', { name: 'Also copy automatic claiming' }),
  ).not.toBeChecked();
  await dialog.getByRole('button', { name: 'Copy mining preferences', exact: true }).click();
  await expect(dialog).not.toBeVisible();
  const copied = await settings(request, third);
  expect(copied.games_to_watch).toEqual((await settings(request, 'primary')).games_to_watch);
  expect(copied.mining_paused).toBe(original.mining_paused);
  expect(copied.auto_claim_drops).toBe(original.auto_claim_drops);
});

test('device sign-in verifies an account before adding it and removal keeps other accounts running', async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto('/settings#account');
  await page.getByRole('button', { name: 'Add account', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Add account', exact: true });
  await expect(dialog.locator('code')).toHaveText('TESTCODE');
  expect((await overview(request)).accounts).toHaveLength(3);
  await dialog.getByRole('button', { name: 'Done', exact: true }).click();
  await expect(dialog.getByRole('textbox', { name: 'Display label', exact: true })).toBeVisible();
  await dialog.getByRole('textbox').fill('Weekend');
  await dialog.getByRole('button', { name: 'Add account', exact: true }).click();
  await expect(dialog).not.toBeVisible();
  await expect.poll(async () => (await overview(request)).accounts.length).toBe(4);
  await page.getByRole('button', { name: 'Remove Account 2', exact: true }).click();
  const actions = page.getByRole('dialog', { name: 'Account 2', exact: true });
  await expect(actions).toContainText('Its local data is kept');
  await actions.getByRole('button', { name: 'Remove account', exact: true }).click();
  await expect.poll(async () => (await overview(request)).accounts.length).toBe(3);
  expect(
    (await overview(request)).accounts.find((account) => account.id === 'primary')?.connected,
  ).toBe(true);
});

test('shared connection changes require save and cannot be written through a scoped settings request', async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto('/settings#connection');
  const quality = page.getByRole('combobox', { name: 'Connection Quality:', exact: true });
  await quality.selectOption('5');
  await switchTo(page, 'Account 2');
  await expect(quality).toHaveValue('5');
  const saved = page.waitForResponse(
    (response) =>
      new URL(response.url()).pathname === '/api/accounts' &&
      response.request().method() === 'POST',
  );
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  expect((await saved).status()).toBe(200);
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
  expect((await settings(request, 'primary')).connection_quality).toBe(5);
  expect((await settings(request, second)).connection_quality).toBe(5);
  const before = await settings(request, second);
  expect(
    (
      await request.post(`/api/accounts/${second}/settings`, {
        headers,
        data: { revision: before.revision, connection_quality: 2 },
      })
    ).status(),
  ).toBe(400);
});

for (const form of ['connection', 'notifications']) {
  test(`an accepted ${form} save stays busy across account switches`, async ({ page, request }) => {
    await seed(request);
    await page.goto(`/settings#${form}`);
    const field =
      form === 'connection'
        ? page.getByRole('combobox', { name: 'Connection Quality:', exact: true })
        : page.getByLabel('Chat ID', { exact: true });
    if (form === 'connection') await field.selectOption('5');
    else await field.fill('42');
    let release!: () => void;
    let accepted = false;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route(
      form === 'connection' ? '**/api/accounts' : '**/api/telegram',
      async (route) => {
        if (route.request().method() !== 'POST') return route.continue();
        const response = await route.fetch();
        accepted = true;
        await held;
        await route.fulfill({ response });
      },
    );
    try {
      await page.getByRole('button', { name: 'Save', exact: true }).click();
      await expect.poll(() => accepted).toBe(true);
      await switchTo(page, 'Account 2');
      await expect(field).toBeDisabled();
      await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
    } finally {
      release();
    }
    await expect(field).toBeEnabled();
    await expect(field).toHaveValue(form === 'connection' ? '5' : '42');
  });
}

test('back navigation and copied campaign links retain their original account', async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto('/campaigns');
  await expect(page).toHaveURL(/account=primary/);
  await switchTo(page, 'Account 2');
  await expect(page).toHaveURL(new RegExp(`account=${second}`));
  await page.goBack();
  await expect(page).toHaveURL(/account=primary/);
  await expect(page.locator('.account-trigger').filter({ visible: true })).toContainText(
    'Twitch 123456',
  );
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await expect(page.locator('#mining-drop-details')).toHaveAttribute('href', /account=primary/);
  await switchTo(page, 'Account 2');
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await expect(page.locator('#mining-drop-details')).toHaveAttribute(
    'href',
    new RegExp(`account=${second}`),
  );
});

test('an unavailable account stops subscriptions and Retry reconnects the same selected profile', async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto(`/settings?account=${second}#account`);
  await expect(page.locator('.account-trigger').filter({ visible: true })).toContainText(
    'Account 2',
  );
  expect(
    (
      await request.post('/__test/event', {
        headers,
        data: { account: second, event: 'account_unavailable', data: null },
      })
    ).ok(),
  ).toBe(true);
  await expect(page.getByRole('button', { name: 'Try again', exact: true })).toBeVisible();
  const connections: string[] = [];
  page.on('websocket', (socket) => {
    if (new URL(socket.url()).searchParams.get('account') === second)
      connections.push(socket.url());
  });
  await page.waitForTimeout(1500);
  expect(connections).toEqual([]);
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Try again', exact: true })).toHaveCount(0);
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await page.getByRole('link', { name: 'Edit', exact: true }).click();
  await expect(
    page.getByRole('checkbox', { name: 'Automatically claim drops', exact: true }),
  ).toBeEnabled();
  await expect(page).toHaveURL(new RegExp(`account=${second}`));
  expect(
    (await overview(request)).accounts.find((account) => account.id === 'primary')?.connected,
  ).toBe(true);
});

test('offline account management disables writes and reconnects without changing the viewed account', async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto(`/settings?account=${second}#account`);
  const add = page.getByRole('button', { name: 'Add account', exact: true });
  await expect(add).toBeEnabled();
  await page.getByRole('checkbox', { name: 'Select all', exact: true }).check();
  const pause = page.getByRole('button', { name: 'Pause', exact: true });
  await expect(pause).toBeEnabled();
  try {
    await page.context().setOffline(true);
    await expect(add).toBeDisabled();
    await expect(pause).toBeDisabled();
  } finally {
    await page.context().setOffline(false);
  }
  await expect(add).toBeEnabled();
  await expect(pause).toBeEnabled();
  await expect(page.locator('.account-trigger').filter({ visible: true })).toContainText(
    'Account 2',
  );
  await expect(page).toHaveURL(new RegExp(`account=${second}`));
});

test('copy source stays valid when an account becomes unavailable or is removed', async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto(`/settings?account=${second}#account`);
  await request.post('/__test/event', {
    headers,
    data: { account: second, event: 'account_unavailable', data: null },
  });
  await expect(page.getByRole('button', { name: 'Try again', exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: 'Select Account 3', exact: true }).check();
  await page.getByRole('button', { name: 'Copy mining preferences', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Copy mining preferences', exact: true });
  const source = dialog.getByRole('combobox');
  await expect(source).toHaveValue('primary');
  const copy = dialog.getByRole('button', { name: 'Copy mining preferences', exact: true });
  await copy.click();
  await expect(dialog).not.toBeVisible();
  expect((await settings(request, third)).games_to_watch).toEqual(
    (await settings(request, 'primary')).games_to_watch,
  );
  await page.getByRole('button', { name: 'Copy mining preferences', exact: true }).click();
  await expect(source).toHaveValue('primary');
  await source.selectOption(third);
  await request.post('/api/accounts', { headers, data: { action: 'remove', id: third } });
  await expect(source).toHaveValue('primary');
  await expect(copy).toBeDisabled();
});

test('Done stays disabled after acceptance while Twitch verification is pending', async ({
  page,
  request,
}) => {
  await page.goto('/settings#account');
  await page.getByRole('button', { name: 'Add account', exact: true }).click();
  let confirmations = 0;
  await page.route('**/api/accounts', async (route) => {
    if (
      route.request().method() === 'POST' &&
      route.request().postDataJSON().action === 'confirm_login'
    ) {
      confirmations++;
      return route.fulfill({ json: {} });
    }
    return route.continue();
  });
  const dialog = page.getByRole('dialog', { name: 'Add account', exact: true });
  const done = dialog.getByRole('button', { name: 'Done', exact: true });
  await expect(
    dialog.getByText('Authorize the Twitch account', { exact: false }),
  ).not.toBeVisible();
  await done.click();
  await expect(done).toBeDisabled();
  await expect(dialog.locator('code')).toHaveText('TESTCODE');
  await page.waitForTimeout(150);
  await expect(done).toBeDisabled();
  expect(confirmations).toBe(1);
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect((await overview(request)).login).toBeNull();
});

test('account actions and expanded profile stay compact on phone and desktop', async ({
  page,
  request,
}) => {
  await seed(request);
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/settings#account');
    const panel = page.locator('#settings-account');
    await expect(panel.locator('summary')).toHaveCount(0);
    await expect(panel.getByText('Twitch profile and badges', { exact: true })).toHaveCount(0);
    await expect(
      panel.getByRole('button', { name: 'Rename Account 2', exact: true }),
    ).toBeVisible();
    await expect(
      panel.getByRole('button', { name: 'Sign out Account 2', exact: true }),
    ).toBeVisible();
    await expect(
      panel.getByRole('button', { name: 'Remove Account 2', exact: true }),
    ).toBeVisible();
    await panel.getByRole('button', { name: 'Rename Account 2', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Account 2', exact: true });
    await expect(dialog.getByRole('textbox', { name: 'Display label', exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  }
});

for (const width of [390, 1280]) {
  test(`catalog key stays private across saves and account switches at ${width}px`, async ({
    page,
    request,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await seed(request);
    await page.goto('/settings#connection');
    const key = page.getByLabel('Sunkwi API key', { exact: true });
    const save = page.getByRole('button', { name: 'Save', exact: true });
    await key.fill('test-browser-catalog-key');
    await expect(key).toHaveAttribute('type', 'password');
    await switchTo(page, 'Account 2');
    await expect(key).toHaveValue('test-browser-catalog-key');
    await save.click();
    await expect(save).toBeDisabled();
    await expect(key).toHaveValue('');
    await expect(key).toHaveAttribute('placeholder', 'Key saved');
    const stored = await (await request.get('/api/connection')).json();
    expect(stored.catalog_api_key_configured).toBe(true);
    expect(JSON.stringify(stored)).not.toContain('test-browser-catalog-key');
    expect(stored.values).not.toHaveProperty('catalog_api_key');
    await page.getByLabel('Inventory refresh interval (minutes)', { exact: true }).fill('17');
    await save.click();
    await expect(save).toBeDisabled();
    await page.reload();
    await expect(key).toHaveAttribute('placeholder', 'Key saved');
    await page
      .getByRole('button', { name: 'Inventory refresh interval (minutes) help', exact: true })
      .click();
    await expect(page.getByRole('note')).toContainText(
      'The public catalog follows its own rate limit.',
    );
    await page.keyboard.press('Escape');
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    await page.screenshot({ path: `../artifacts/catalog-connection-${width}.png` });
    await page.getByRole('button', { name: 'Remove Sunkwi API key', exact: true }).click();
    await expect(key).toHaveAttribute('placeholder', 'Key will be removed on save');
    await save.click();
    await expect(save).toBeDisabled();
    expect((await (await request.get('/api/connection')).json()).catalog_api_key_configured).toBe(
      false,
    );
  });
}
