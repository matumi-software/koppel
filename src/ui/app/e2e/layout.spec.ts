import { expect, test, type Page } from '@playwright/test';
import { answerSignedOut, serve, signedOut, wire } from './support';

// How the screens use the window, with the service's answers stood in (see support.ts).

/** The middle of an element, and of the panel it is in, across and down, in pixels. */
async function centres(page: Page, selector: string) {
  return page.evaluate((sel) => {
    const box = document.querySelector(sel)!.getBoundingClientRect();
    const panel = document.querySelector('main')!.getBoundingClientRect();
    return {
      item: box.left + box.width / 2,
      panel: panel.left + panel.width / 2,
      itemDown: box.top + box.height / 2,
      panelDown: panel.top + panel.height / 2,
    };
  }, selector);
}

test.describe('the signed-out screens', () => {
  test('the sign-in is in the middle of the window', async ({ page }) => {
    await answerSignedOut(page, { ...signedOut, passwordSet: true, setupOpen: false });
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Sign in to bench-pc' })).toBeVisible();

    const { item, panel, itemDown, panelDown } = await centres(page, '.hw-tile');
    expect(Math.abs(item - panel)).toBeLessThanOrEqual(2);
    // The sign-in's column holds the tile and a mark above it, so the tile sits a little below the middle.
    expect(Math.abs(itemDown - panelDown)).toBeLessThanOrEqual(80);
  });

  test('the first password is in the middle of the window', async ({ page }) => {
    await answerSignedOut(page, signedOut);
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Set up bench-pc' })).toBeVisible();

    const { item, panel, itemDown, panelDown } = await centres(page, '.kp-form');
    expect(Math.abs(item - panel)).toBeLessThanOrEqual(2);
    expect(Math.abs(itemDown - panelDown)).toBeLessThanOrEqual(2);
  });
});

test.describe('the overview on a wide screen', () => {
  test.skip(({ isMobile }) => isMobile, 'a wide-screen check');
  test.use({ viewport: { width: 1920, height: 1000 } });

  for (const role of ['server', 'client', 'both'] as const) {
    test(`a ${role} fills the width of the page`, async ({ page }, info) => {
      await serve(page, role);
      await page.goto('/#overview');
      await expect(page.locator('.hw-tile').first()).toBeVisible();
      await page.waitForTimeout(500);
      await page.screenshot({ path: info.outputPath(`overview-${role}.png`) });

      // Every row of tiles reaches the right edge of the grid, so no column is left empty.
      const gaps = await page.evaluate(() => {
        const grid = document.querySelector('.hw-tile-grid')!.getBoundingClientRect();
        const rows = new Map<number, number>();
        document.querySelectorAll('.hw-tile-grid > *').forEach((tile) => {
          const box = tile.getBoundingClientRect();
          const top = Math.round(box.top);
          rows.set(top, Math.max(rows.get(top) ?? 0, box.right));
        });
        return [...rows.values()].map((right) => Math.round(grid.right - right));
      });
      expect(gaps.length).toBeGreaterThan(0);
      for (const gap of gaps) expect(gap).toBeLessThanOrEqual(2);
    });
  }
});

test.describe('the pairing requests on a phone', () => {
  test.skip(({ isMobile }) => !isMobile, 'a phone check');

  test('the code field of a waiting request is wide enough to type eight digits', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await serve(page, 'server');
    await page.goto('/#clients');
    const field = page.getByLabel(/^Code shown on/).first();
    await expect(field).toBeVisible();
    // It once shrank to 12px beside the Approve button, so the digits could not be seen.
    expect((await field.boundingBox())!.width).toBeGreaterThan(120);
  });
});

test.describe('the servers table on a phone', () => {
  test.skip(({ isMobile }) => !isMobile, 'a phone check');

  test('the link of each server is readable beside its pinned actions, even with a long address', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    const first = wire('servers').servers[0];
    const long = `https://${'cellar-with-a-long-host-name.'.repeat(3)}example.net:5241`;
    const servers = [first, { ...first, id: 'server-2', name: null, serverUrl: long }];
    const devices = wire('client-devices');
    await serve(page, 'client', {
      answers: {
        '/api/v1/client/servers': { servers },
        '/api/v1/client/devices': { ...devices, servers: servers.map((s, i) => ({ id: s.id, name: s.name, link: i ? 'error' : 'connected', linkMessage: null, devices: [] })) },
      },
    });
    await page.goto('/#connection');
    const table = page.locator('[aria-label="Servers this computer attaches from"]');
    await expect(table).toBeVisible();
    await page.waitForTimeout(600);

    // The Link column is gone: the status sits under the name, which wraps, so nothing needs a sideways scroll
    // and the pinned actions are not over it.
    const m = await table.evaluate((el) => {
      const wrap = el.closest('.hw-table-wrap') ?? el.parentElement!;
      const action = el.querySelector('tbody button')!.getBoundingClientRect();
      const statuses = [...el.querySelectorAll('tbody .hw-status')].map((s) => s.getBoundingClientRect().right);
      return { scrolls: wrap.scrollWidth - wrap.clientWidth, actionLeft: action.left, statuses };
    });
    expect(m.scrolls, 'the table scrolls sideways').toBeLessThanOrEqual(1);
    expect(m.statuses.length, 'each row shows its link').toBe(2);
    for (const right of m.statuses) expect(right, 'the link runs under the pinned actions').toBeLessThanOrEqual(m.actionLeft);
    await expect(page.getByText('Cannot reach')).toBeVisible();
  });
});
