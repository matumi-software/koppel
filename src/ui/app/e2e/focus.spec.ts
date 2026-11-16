import { expect, test, type Page } from '@playwright/test';
import { serve } from './support';

// After a person goes to a page, focus moves to its heading (so a screen reader starts there). A mouse user must
// not see a ring around that heading; a keyboard user should.
async function heading(page: Page) {
  return page.evaluate(() => {
    const h1 = document.querySelector('main h1')!;
    const style = getComputedStyle(h1);
    return {
      focused: document.activeElement === h1,
      focusVisible: h1.matches(':focus-visible'),
      ring: style.outlineStyle !== 'none' && parseFloat(style.outlineWidth) > 0,
      text: h1.textContent,
    };
  });
}

test.describe('the page heading after going to a page', () => {
  test.skip(({ isMobile }) => isMobile, 'the phone goes to a page by its bottom bar');
  test('a mouse click leaves no ring', async ({ page }) => {
    await serve(page, 'both');
    await page.goto('/#overview');
    await expect(page.locator('main h1')).toBeVisible();

    await page.getByRole('link', { name: 'Clients' }).first().click();
    await expect(page.locator('main h1')).toHaveText(/Clients of/);

    const after = await heading(page);
    expect(after.focused).toBe(true);
    expect(after.ring).toBe(false);
  });

  test('the keyboard shows where focus went', async ({ page }) => {
    await serve(page, 'both');
    await page.goto('/#overview');
    await expect(page.locator('main h1')).toBeVisible();

    await page.getByRole('link', { name: 'Clients' }).first().focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('main h1')).toHaveText(/Clients of/);

    const after = await heading(page);
    expect(after.focused).toBe(true);
    expect(after.ring).toBe(true);
  });
});
