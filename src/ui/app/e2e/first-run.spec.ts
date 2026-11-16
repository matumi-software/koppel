import { expect, test, type Page } from '@playwright/test';

// The first screens a person sees, with the service's answers stood in by the test, so no Koppel needs to
// run. The page must say what it is for and must not scroll sideways at either width.
const answer = (page: Page, state: object) =>
  page.route('**/api/v1/auth/state', (route) => route.fulfill({ json: state }));

const state = {
  signedIn: false,
  passwordSet: false,
  machineName: 'bench-pc',
  setupOpen: true,
  setupTokenRequired: false,
  setupUntil: '2026-01-01T09:00:00Z',
};

async function sideways(page: Page) {
  return page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
}

test('a new install asks for the first password', async ({ page }) => {
  await answer(page, state);
  await page.goto('/');

  await expect(page.getByRole('heading', { name: 'Set up bench-pc' })).toBeVisible();
  await expect(page.getByLabel('New password')).toBeVisible();
  expect(await sideways(page)).toBeLessThanOrEqual(0);
});

test('an install with a password asks for it', async ({ page }) => {
  await answer(page, { ...state, passwordSet: true, setupOpen: false });
  await page.goto('/');

  await expect(page.getByRole('heading', { name: 'Sign in to bench-pc' })).toBeVisible();
  await expect(page.locator('input[type=password]')).toBeVisible();
  expect(await sideways(page)).toBeLessThanOrEqual(0);
});
