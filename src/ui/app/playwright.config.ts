import { defineConfig, devices } from '@playwright/test';

// Browsers are not in this project: they live once on the machine, in the folder PLAYWRIGHT_BROWSERS_PATH
// names (see docs/ui.md, "Testing in a browser"). Chromium only, at desktop and phone width.
const port = Number(process.env.DEV_SERVER_PORT ?? 61040);

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  reporter: 'list',
  use: {
    baseURL: `https://localhost:${port}`,
    // The dev server is HTTPS with a self-signed certificate, as Koppel itself is.
    ignoreHTTPSErrors: true,
  },
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'], viewport: { width: 1120, height: 720 } } },
    { name: 'phone', use: { ...devices['Pixel 7'] } },
  ],
  webServer: {
    command: 'pnpm dev',
    url: `https://localhost:${port}`,
    ignoreHTTPSErrors: true,
    reuseExistingServer: !process.env.CI,
  },
});
