import { expect, test } from '@playwright/test';
import { serve } from './support';

// The label keeps its room and the value takes the rest, cut with an ellipsis (Heartwood's value rule). The old
// package cut it to 37 of 95 characters with 500 px free; it must now use the room it has.
test('the fingerprint uses the room a wide screen gives it', async ({ page, isMobile }) => {
  test.skip(isMobile, 'the wide screen is the desktop project');
  await serve(page, 'both');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#security');
  const row = page.getByRole('group', { name: 'Server certificate' }).or(page.locator('main')).first();
  await expect(row.getByText('Fingerprint', { exact: true }).first()).toBeVisible();
  const cut = await page.evaluate(() => {
    const value = [...document.querySelectorAll('main *')].find((el) => el.children.length === 0 && /^[0-9A-Fa-f:]{40,}$/.test(el.textContent?.trim() ?? ''));
    if (!value) return { found: false as const };
    const box = value.getBoundingClientRect();
    return { found: true as const, cut: value.scrollWidth > value.clientWidth + 1, width: Math.round(box.width), text: value.textContent!.length };
  });
  expect(cut, JSON.stringify(cut)).toMatchObject({ found: true });
  expect(cut.found && cut.width, JSON.stringify(cut)).toBeGreaterThan(500);
});
