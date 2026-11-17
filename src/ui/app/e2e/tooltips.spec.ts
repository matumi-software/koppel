import { expect, test } from '@playwright/test';
import { serve, wire } from './support';

// What shows the rest of a value that is cut: a tooltip, on hover and, for a list row, while the row has the
// keyboard focus. screens.spec.ts hovers every cut value it finds; these are the named cases.
test.skip(({ isMobile }) => isMobile, 'needs a pointer');
test.use({ viewport: { width: 1120, height: 720 } });

const holder = 'a-computer-with-a-very-long-name-that-does-not-fit-in-the-row-at-all-'.repeat(6);

test('a list row sub line that is cut shows its full text on hover and on keyboard focus', async ({ page }) => {
  const devices = wire('server-devices');
  await serve(page, 'server', { answers: { '/api/v1/server/devices': { ...devices, devices: [{ ...devices.devices[0], attachedTo: holder }, devices.devices[1]] } } });
  await page.goto('/#overview');
  const sub = page.locator('span.hw-list-sub', { hasText: holder.slice(0, 20) }).first();
  await expect(sub).toBeVisible();
  expect(await sub.evaluate((el) => el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1), 'the sub line is cut').toBe(true);

  await sub.hover();
  await expect(page.getByRole('tooltip').filter({ hasText: holder })).toBeVisible();
  await page.mouse.move(0, 0);
  await expect(page.getByRole('tooltip')).toHaveCount(0);

  // The row is what takes the focus, and its tooltip shows with it.
  await sub.locator('xpath=ancestor::*[contains(@class,"hw-list-row")][1]').focus();
  await expect(page.getByRole('tooltip').filter({ hasText: holder })).toBeVisible();
});

test('a fingerprint that is cut shows all of it on hover', async ({ page }) => {
  await serve(page, 'server');
  await page.goto('/#security');
  const value = page.locator('span.hw-ellip').first();
  await expect(value).toBeVisible();
  await value.hover();
  const fingerprint = wire('system-info').fingerprint as string;
  await expect(page.getByRole('tooltip').filter({ hasText: fingerprint.slice(0, 20) })).toBeVisible();
});
