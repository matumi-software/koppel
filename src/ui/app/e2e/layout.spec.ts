import { expect, test, type Page } from '@playwright/test';
import { answerSignedOut, serve, signedOut } from './support';

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
