import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

// How the screens use the window. The service's answers are stood in by the test (the same JSON the unit
// tests and the Rust wire test share), so no Koppel needs to run.
const wire = (name: string) => JSON.parse(readFileSync(new URL(`../src/test/wire/${name}.json`, import.meta.url), 'utf8'));

const signedOut = { ...wire('auth-state'), setupTokenRequired: false };

async function answerSignedOut(page: Page, state: object) {
  await page.route('**/api/v1/auth/state', (route) => route.fulfill({ json: state }));
}

/** The middle of an element, and of the panel it is in, in pixels from the left. */
async function centres(page: Page, selector: string) {
  return page.evaluate((sel) => {
    const box = document.querySelector(sel)!.getBoundingClientRect();
    const panel = document.querySelector('main')!.getBoundingClientRect();
    return { item: box.left + box.width / 2, panel: panel.left + panel.width / 2 };
  }, selector);
}

test.describe('the signed-out screens', () => {
  test('the sign-in is in the middle of the window', async ({ page }) => {
    await answerSignedOut(page, { ...signedOut, passwordSet: true, setupOpen: false });
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Sign in to bench-pc' })).toBeVisible();

    const { item, panel } = await centres(page, '.hw-tile');
    expect(Math.abs(item - panel)).toBeLessThanOrEqual(2);
  });

  test('the first password is in the middle of the window', async ({ page }) => {
    await answerSignedOut(page, signedOut);
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Set up bench-pc' })).toBeVisible();

    const { item, panel } = await centres(page, '.kp-form');
    expect(Math.abs(item - panel)).toBeLessThanOrEqual(2);
  });
});

/** A server whose screens the Overview needs, answered from the fixtures. */
async function serve(page: Page, role: 'server' | 'client' | 'both') {
  const settings = { ...wire('settings'), role, setupComplete: true };
  const servers = wire('servers');
  const answers: Record<string, unknown> = {
    '/api/v1/auth/state': { ...signedOut, signedIn: true, passwordSet: true, setupOpen: false },
    '/api/v1/settings': settings,
    '/api/v1/system/info': { ...wire('system-info'), role },
    '/api/v1/system/checks': wire('system-checks').map((c: object) => ({ ...c, ok: true })),
    '/api/v1/activity': [wire('activity-event')],
    '/api/v1/server/clients': wire('clients'),
    '/api/v1/server/devices': wire('server-devices'),
    '/api/v1/server/enrollment': wire('enrollment-view'),
    '/api/v1/server/keys': wire('keys-view'),
    '/api/v1/server/identity': wire('server-identity'),
    '/api/v1/server/pairing': wire('pairing-overview'),
    '/api/v1/client/servers': servers,
    '/api/v1/client/devices': wire('client-devices'),
    '/api/v1/client/discovery': wire('discovered-servers'),
  };
  const push: [string, unknown][] = [
    ['SettingsChanged', settings],
    ...(role !== 'client'
      ? ([['ClientsChanged', answers['/api/v1/server/clients']], ['ServerKeysChanged', answers['/api/v1/server/keys']], ['PairingRequestsChanged', answers['/api/v1/server/pairing']], ['EnrollmentChanged', answers['/api/v1/server/enrollment']], ['DevicesChanged', answers['/api/v1/server/devices']]] as [string, unknown][])
      : []),
    ...(role !== 'server'
      ? ([['ServersChanged', servers], ['ServersDiscovered', answers['/api/v1/client/discovery']], ['ClientDevicesChanged', answers['/api/v1/client/devices']]] as [string, unknown][])
      : []),
  ];
  await page.route('**/api/v1/**', (route) => {
    const { pathname } = new URL(route.request().url());
    if (pathname === '/api/v1/events') {
      const body = push.map(([name, data]) => `event: ${name}\ndata: ${JSON.stringify(data)}\n\n`).join('');
      return route.fulfill({ status: 200, headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-store' }, body });
    }
    return route.fulfill({ json: answers[pathname] ?? [] });
  });
}

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
