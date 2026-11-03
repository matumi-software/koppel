import { fileURLToPath, URL } from 'node:url';

import { defineConfig } from 'vite';
import plugin from '@vitejs/plugin-react';
import fs from 'fs';
import path from 'path';
import child_process from 'child_process';
import { env } from 'process';

const baseFolder =
    env.APPDATA !== undefined && env.APPDATA !== ''
        ? `${env.APPDATA}/ASP.NET/https`
        : `${env.HOME}/.aspnet/https`;

const certificateName = "Koppel.UI";
const certFilePath = path.join(baseFolder, `${certificateName}.pem`);
const keyFilePath = path.join(baseFolder, `${certificateName}.key`);

// The dev certificate is only needed for the dev server, not for `vite build`.
const serving = process.argv.every((arg) => arg !== 'build');

if (serving) {
    if (!fs.existsSync(baseFolder)) {
        fs.mkdirSync(baseFolder, { recursive: true });
    }

    if (!fs.existsSync(certFilePath) || !fs.existsSync(keyFilePath)) {
        if (0 !== child_process.spawnSync('dotnet', [
            'dev-certs',
            'https',
            '--export-path',
            certFilePath,
            '--format',
            'Pem',
            '--no-password',
        ], { stdio: 'inherit', }).status) {
            throw new Error("Could not create certificate.");
        }
    }
}

// Koppel's UI port, HTTPS with Koppel's own self-signed certificate, so the proxy below does not verify it.
const target = env.KOPPEL_UI_URL ?? 'https://127.0.0.1:5240';

export default defineConfig({
    plugins: [plugin()],
    resolve: {
        alias: {
            '@': fileURLToPath(new URL('./src', import.meta.url))
        },
        dedupe: ['react', 'react-dom'],
    },
    server: {
        proxy: {
            // REST and the SignalR hub both live under /api/.
            '^/api/': {
                target,
                secure: false,
                ws: true,
                // Tells Koppel the browser used https, so its same-origin check on Origin passes.
                xfwd: true,
            }
        },
        port: parseInt(env.DEV_SERVER_PORT || '61040'),
        https: serving ? {
            key: fs.readFileSync(keyFilePath),
            cert: fs.readFileSync(certFilePath),
        } : undefined,
    }
})
