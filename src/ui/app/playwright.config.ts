import { defineConfig, devices } from '@playwright/test';

// Browsers are not in this project: they live once on the machine, in the folder PLAYWRIGHT_BROWSERS_PATH
// names (see docs/ui.md, "Testing in a browser"). Chromium only, at desktop and phone width.
// Its own port, below the range the operating system hands to outgoing connections (Windows starts at 49152),
// so nothing else holds it, and --strictPort so a taken port fails at once instead of moving the server.
const port = Number(process.env.E2E_PORT ?? 41040);

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
    command: 'pnpm dev --strictPort',
    env: { DEV_SERVER_PORT: String(port) },
    url: `https://localhost:${port}`,
    ignoreHTTPSErrors: true,
    reuseExistingServer: !process.env.CI,
  },
});
