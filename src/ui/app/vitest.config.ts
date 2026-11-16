import { fileURLToPath, URL } from 'node:url';

import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// Apart from vite.config.ts, which makes a dev certificate and starts a proxy that tests do not want.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
    dedupe: ['react', 'react-dom'],
  },
  test: {
    // The browser tests under e2e/ are Playwright's, run with `pnpm e2e`.
    include: ['src/**/*.test.{ts,tsx}'],
    environment: 'jsdom',
    setupFiles: ['./vitest.setup.ts'],
    css: false,
    // Heartwood ships ES modules with extensionless imports, which only a bundler resolves: let Vite process it.
    server: { deps: { inline: [/@matumi\//] } },
  },
});
