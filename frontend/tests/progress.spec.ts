import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };

for (const width of [1440, 390]) {
  test(`progress time format persists across mining and campaign details at ${width}px`, async ({
    page,
    request,
  }) => {
    await request.post('/__test/reset', { headers, data: {} });
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const progress = {
      ...fixture.current_drop,
      confirmed_minutes: 330,
      current_minutes: 335,
      required_minutes: 360,
    };
    const campaigns = structuredClone(fixture.campaigns);
    Object.assign(campaigns[0]!.drops[0]!, {
      confirmed_minutes: 330,
      current_minutes: 335,
      required_minutes: 360,
    });
    await request.post('/__test/event', {
      headers,
      data: { event: 'inventory_batch_update', data: { campaigns } },
    });
    await request.post('/__test/event', {
      headers,
      data: { event: 'drop_progress', data: progress },
    });
    await page.goto('/');
    const mining = page.getByRole('region', { name: 'Now mining', exact: true });
    await expect(mining.getByText('330 / 360 min', { exact: true })).toBeVisible();
    await page.goto('/?edit=priorities');
    const setting = page.getByRole('combobox', { name: 'Progress time format', exact: true });
    await expect(setting).toHaveValue('minutes');
    await setting.selectOption('hours_minutes');
    await page.getByRole('checkbox', { name: 'Show progress percentage' }).check();
    await expect
      .poll(async () => {
        const saved = await (await request.get('/api/settings')).json();
        return [saved.progress_time_format, saved.show_progress_percentage];
      })
      .toEqual(['hours_minutes', true]);
    await page.screenshot({ path: `../artifacts/progress-time-settings-${width}.png` });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.getByRole('link', { name: 'Back to Mining', exact: true }).click();
    await expect(mining.getByText('5h 30m / 6h (91.6%)', { exact: true })).toBeVisible();
    await expect(mining.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '330');
    await expect(mining.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '360');
    await expect(page).toHaveTitle('91.6% Rust - Drops Miner');
    await page.reload();
    await expect(mining.getByText('5h 30m / 6h (91.6%)', { exact: true })).toBeVisible();
    await page.screenshot({ path: `../artifacts/progress-time-${width}.png` });
    await page.goto('/campaigns?campaign=campaign-1');
    const reward = page
      .getByRole('complementary', { name: 'Campaign details' })
      .locator('#drop-reward-1');
    await expect(reward.getByText('5h 30m / 6h (91.6%)', { exact: true })).toBeVisible();
    await expect(reward.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '330');
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    Object.assign(campaigns[0]!.drops[0]!, { confirmed_at: null });
    await request.post('/__test/event', {
      headers,
      data: { event: 'inventory_batch_update', data: { campaigns } },
    });
    await expect(reward.getByText('0m / 6h', { exact: true })).toBeVisible();
    await request.post('/__test/event', {
      headers,
      data: { event: 'drop_progress', data: { ...progress, confirmed_at: null } },
    });
    await page.goto('/');
    await expect(mining.getByText('0m / 6h', { exact: true })).toBeVisible();
    await expect(mining.getByText(/\d+%/)).toHaveCount(0);
    await page.goto('/?edit=priorities');
    await expect(setting).toHaveValue('hours_minutes');
    await setting.selectOption('minutes');
    await expect
      .poll(async () => (await (await request.get('/api/settings')).json()).progress_time_format)
      .toBe('minutes');
    await page.getByRole('link', { name: 'Back to Mining', exact: true }).click();
    await expect(mining.getByText('0 / 360 min', { exact: true })).toBeVisible();
  });

  test(`progress percentage is saved from Mining preferences at ${width}px`, async ({
    page,
    request,
  }) => {
    await request.post('/__test/reset', { headers, data: {} });
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/');
    const mining = page.getByRole('region', { name: 'Now mining', exact: true });
    await expect(mining.getByText('42 / 60 min', { exact: true })).toBeVisible();
    await page.goto('/?edit=priorities');
    const setting = page.getByRole('checkbox', { name: 'Show progress percentage' });
    await expect(setting).not.toBeChecked();
    await setting.check();
    await expect
      .poll(
        async () => (await (await request.get('/api/settings')).json()).show_progress_percentage,
      )
      .toBe(true);
    await page.getByRole('link', { name: 'Back to Mining', exact: true }).click();
    await expect(mining.getByText('42 / 60 min (70%)', { exact: true })).toBeVisible();
    await page.reload();
    await expect(mining.getByText('42 / 60 min (70%)', { exact: true })).toBeVisible();
    await request.post('/__test/event', {
      headers,
      data: {
        event: 'drop_progress',
        data: { ...fixture.current_drop, confirmed_minutes: 59, current_minutes: 60 },
      },
    });
    await expect(mining.getByText('59 / 60 min (98.3%)', { exact: true })).toBeVisible();
    await expect(page).toHaveTitle('98.3% Rust - Drops Miner');
    for (const minutes of [501, 502, 503]) {
      await request.post('/__test/event', {
        headers,
        data: {
          event: 'drop_progress',
          data: {
            ...fixture.current_drop,
            confirmed_minutes: minutes,
            current_minutes: 600,
            required_minutes: 1000,
          },
        },
      });
      await expect(
        mining.getByText(`${minutes} / 1000 min (${minutes / 10}%)`, { exact: true }),
      ).toBeVisible();
      await expect(page).toHaveTitle(`${minutes / 10}% Rust - Drops Miner`);
      await expect(mining.getByRole('progressbar')).toHaveAttribute('aria-valuenow', `${minutes}`);
      await expect(mining.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '1000');
    }
    await page.screenshot({ path: `../artifacts/progress-percentage-${width}.png` });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await request.post('/__test/event', {
      headers,
      data: { event: 'drop_progress', data: { ...fixture.current_drop, confirmed_at: null } },
    });
    await expect(mining.getByText('0 / 60 min', { exact: true })).toBeVisible();
    await expect(mining.getByText(/\d+%/)).toHaveCount(0);
    await page.goto('/?edit=priorities');
    await expect(setting).toBeChecked();
    await setting.uncheck();
    await expect
      .poll(
        async () => (await (await request.get('/api/settings')).json()).show_progress_percentage,
      )
      .toBe(false);
    await request.post('/__test/event', {
      headers,
      data: { event: 'drop_progress', data: fixture.current_drop },
    });
    await page.getByRole('link', { name: 'Back to Mining', exact: true }).click();
    await expect(mining.getByText('42 / 60 min', { exact: true })).toBeVisible();
  });
}
