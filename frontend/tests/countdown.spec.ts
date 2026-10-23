import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };

for (const width of [1440, 390]) {
  test(`independent progress displays persist and stay accessible at ${width}px`, async ({
    page,
    request,
  }) => {
    await request.post('/__test/reset', { headers, data: {} });
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/?edit=priorities');
    const percentage = page.getByRole('checkbox', {
      name: 'Show progress percentage',
      exact: true,
    });
    const time = page.getByRole('checkbox', { name: 'Show watched / required time', exact: true });
    const countdown = page.getByRole('checkbox', { name: 'Show countdown', exact: true });
    await expect(time).toBeChecked();
    await expect(percentage).not.toBeChecked();
    await expect(countdown).not.toBeChecked();
    for (const p of [false, true])
      for (const m of [false, true])
        for (const c of [false, true]) {
          await percentage.setChecked(p);
          await time.setChecked(m);
          await countdown.setChecked(c);
          await expect
            .poll(async () => {
              const saved = await (await request.get('/api/settings')).json();
              return [
                saved.show_progress_percentage,
                saved.show_progress_time,
                saved.show_progress_countdown,
              ];
            })
            .toEqual([p, m, c]);
          await expect(
            page.getByRole('combobox', { name: 'Progress time format', exact: true }),
          ).toBeEnabled({ enabled: m });
          await page.getByRole('link', { name: 'Back to Mining', exact: true }).click();
          const mining = page.getByRole('region', { name: 'Now mining', exact: true });
          const label = m ? `42 / 60 min${p ? ' (70%)' : ''}` : p ? '70%' : null;
          if (label) await expect(mining.getByText(label, { exact: true })).toBeVisible();
          else await expect(mining.locator('.tabular-nums')).toHaveCount(c ? 1 : 0);
          await expect(mining.getByText(/~\d+:\d{2}:\d{2} left/)).toHaveCount(c ? 1 : 0);
          await expect(mining.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '42');
          await page.goto('/?edit=priorities');
        }
    await page.screenshot({ path: `../artifacts/progress-display-settings-${width}.png` });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.getByRole('link', { name: 'Back to Mining', exact: true }).click();
    await page.reload();
    const mining = page.getByRole('region', { name: 'Now mining', exact: true });
    await expect(mining.getByText('42 / 60 min (70%)', { exact: true })).toBeVisible();
    await expect(mining.getByText(/~\d+:\d{2}:\d{2} left/)).toBeVisible();
    await page.screenshot({ path: `../artifacts/progress-display-${width}.png` });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.goto('/campaigns?campaign=campaign-1');
    const reward = page.locator('#drop-reward-1');
    await expect(reward.getByText('42 / 60 min (70%)', { exact: true })).toBeVisible();
    await expect(reward.getByText(/~\d+:\d{2}:\d{2} left/)).toBeVisible();
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
}

test('countdown ticks, pauses, corrects, survives navigation and bounds stale estimates', async ({
  page,
  request,
  context,
}) => {
  await request.post('/__test/reset', { headers, data: {} });
  await request.post('/api/settings', { headers, data: { show_progress_countdown: true } });
  await page.clock.install();
  await page.goto('/');
  const mining = page.locator('section[aria-labelledby="mining-heading"]');
  const remaining = mining.getByText(/~\d+:\d{2}:\d{2} left/);
  await expect(remaining).toHaveText('~00:12:00 left');
  await page.clock.runFor(5000);
  await expect(remaining).toHaveText('~00:11:55 left');
  await page.getByRole('button', { name: 'Pause mining', exact: true }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).mining_paused)
    .toBe(true);
  await page.clock.runFor(5000);
  await expect(remaining).toHaveText('~00:11:55 left');
  await page.getByRole('button', { name: 'Resume mining', exact: true }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).mining_paused)
    .toBe(false);
  await page.clock.runFor(5000);
  await expect(remaining).toHaveText('~00:11:50 left');
  await page.getByRole('link', { name: 'Campaigns', exact: true }).first().click();
  await page.clock.runFor(5000);
  await page.getByRole('link', { name: 'Mining', exact: true }).first().click();
  await expect(remaining).toHaveText('~00:11:45 left');
  await page.clock.runFor(300000);
  await expect(remaining).toHaveText('~00:11:01 left');
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'drop_progress',
      data: {
        ...fixture.current_drop,
        current_minutes: 49,
        remaining_seconds: 660,
      },
    },
  });
  await expect(remaining).toHaveText('~00:11:00 left');
  await context.setOffline(true);
  await request.post('/__test/reconnect', { headers, data: {} });
  await expect(page.getByRole('button', { name: 'Pause mining', exact: true })).toBeDisabled();
  const frozen = await remaining.textContent();
  await page.clock.runFor(5000);
  await expect(remaining).toHaveText(frozen!);
  await context.setOffline(false);
});

test('unknown progress has no invented countdown and zero never claims a reward', async ({
  page,
  request,
}) => {
  await request.post('/__test/reset', { headers, data: {} });
  await request.post('/api/settings', { headers, data: { show_progress_countdown: true } });
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'drop_progress',
      data: {
        ...fixture.current_drop,
        confirmed_at: null,
      },
    },
  });
  await page.goto('/');
  const mining = page.getByRole('region', { name: 'Now mining', exact: true });
  await expect(mining.getByText('--:--:--', { exact: true })).toBeVisible();
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'drop_progress',
      data: {
        ...fixture.current_drop,
        current_minutes: 60,
        remaining_seconds: 0,
      },
    },
  });
  await expect(mining.getByText('~00:00:00 left', { exact: true })).toBeVisible();
  await expect(mining.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '42');
  await expect(mining.getByText('Watch time complete', { exact: true })).toHaveCount(0);
});
