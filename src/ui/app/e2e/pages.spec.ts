import { expect, test, type Page } from '@playwright/test';
import { serve } from './support';

// What every page does with the width it has, on the desktop and the phone project: nothing runs past the
// edge, and a page is readable when it opens (a panel that is a sheet on a phone is not open on arrival).
const pages = ['overview', 'devices', 'remote', 'clients', 'activity', 'check', 'connection', 'security', 'settings'];

// The narrowest phone people use; the phone project's own device is a little wider.
test.beforeEach(async ({ page, isMobile }) => {
  if (isMobile) await page.setViewportSize({ width: 390, height: 844 });
});

async function overflow(page: Page) {
  return page.evaluate(() => {
    const edge = document.documentElement.clientWidth;
    const main = document.querySelector('main')!;
    // A table inside a box that scrolls sideways is meant to be wider than the screen: that box is what must fit.
    const scrolls = (el: Element) => { for (let up = el.parentElement; up && up !== main; up = up.parentElement) { if (/auto|scroll/.test(getComputedStyle(up).overflowX)) return up; } return null; };
    const wide = [...main.querySelectorAll('table, [role="grid"], [role="table"]')]
      .map((el) => ({ what: el.getAttribute('aria-label') ?? el.tagName, right: Math.round((scrolls(el) ?? el).getBoundingClientRect().right) }))
      .filter((el) => el.right > edge + 1);
    return { edge, page: document.documentElement.scrollWidth - edge, main: main.scrollWidth - main.clientWidth, wide };
  });
}

for (const name of pages) {
  test(`${name} fits the width`, async ({ page }) => {
    await serve(page, 'both');
    await page.goto(`/#${name}`);
    await expect(page.locator('main h1, main [role="heading"]').first()).toBeVisible();
    await page.waitForTimeout(300);

    const measured = await overflow(page);
    expect(measured.page, `the page scrolls sideways: ${JSON.stringify(measured)}`).toBeLessThanOrEqual(0);
    expect(measured.main, `the content scrolls sideways: ${JSON.stringify(measured)}`).toBeLessThanOrEqual(1);
    expect(measured.wide, `a table runs past the edge: ${JSON.stringify(measured)}`).toEqual([]);
  });
}

test('the connection page is readable when it opens', async ({ page }) => {
  await serve(page, 'both');
  await page.goto('/#connection');
  const heading = page.locator('main h1').first();
  await expect(heading).toBeVisible();
  await page.waitForTimeout(800);

  // On a phone the settings panel is a sheet, and it is open only when a person asks for it.
  if (await page.evaluate(() => document.documentElement.clientWidth < 800)) {
    expect(await page.locator('[class*="hw-inspector"]').count(), 'the settings panel is open on arrival').toBe(0);
  }

  // Whatever is on top of the heading's middle is the heading, not a panel over the page.
  const covered = await heading.evaluate((el) => {
    const box = el.getBoundingClientRect();
    const top = document.elementFromPoint(box.left + 8, box.top + box.height / 2);
    return !(top && (top === el || el.contains(top)));
  });
  expect(covered, 'something covers the page heading').toBe(false);
});

test('the form to add a server opens and fits', async ({ page }) => {
  await serve(page, 'both');
  await page.goto('/#connection');
  await page.getByRole('button', { name: 'Add a server' }).click();

  await expect(page.getByLabel('Server address')).toBeVisible();
  await expect(page.getByLabel('Pairing code')).toBeVisible();
  // The form is about a server that is not saved yet, so no other server's settings panel sits beside it.
  expect(await page.locator('[class*="hw-inspector"]').count(), 'a settings panel is open beside the form').toBe(0);
  const measured = await overflow(page);
  expect(measured.page).toBeLessThanOrEqual(0);
  expect(measured.wide).toEqual([]);
});
