import { fileURLToPath, URL } from 'node:url';

import { defineConfig } from 'vite';
import plugin from '@vitejs/plugin-react';
import basicSsl from '@vitejs/plugin-basic-ssl';
import { env } from 'process';

// Koppel's UI port, HTTPS with Koppel's own self-signed certificate, so the proxy below does not verify it.
const target = env.KOPPEL_UI_URL ?? 'https://127.0.0.1:5240';

export default defineConfig({
    // The dev server is HTTPS too, with a self-signed certificate the plugin makes once and keeps in
    // node_modules/.vite/basic-ssl, so the browser warns the first time as it does for Koppel itself.
    // `vite build` does not use it.
    plugins: [plugin(), basicSsl({ name: 'Koppel.UI', domains: ['localhost', '127.0.0.1'] })],
    resolve: {
        alias: {
            '@': fileURLToPath(new URL('./src', import.meta.url))
        },
        dedupe: ['react', 'react-dom'],
    },
    server: {
        proxy: {
            // REST and the live push both live under /api/.
            '^/api/': {
                target,
                secure: false,
                // Tells Koppel the browser used https, so its same-origin check on Origin passes.
                xfwd: true,
            }
        },
        port: parseInt(env.DEV_SERVER_PORT || '61040'),
    }
})
