import type { ConnectionView } from '@/api/client';
import type { KoppelSettings } from '@/api/settings';
import { ConfigurationPage } from '@/settings/configuration-page';
import { FrameContext, type FrameApi } from '@/shell/frame-context';
import { json } from './harness';

export const settings: KoppelSettings = { role: 'server', shareOnStartup: false, setupComplete: true, notifyPairing: true, notifyLink: true, notifyAttachFailures: true };
export const fingerprint = 'AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99';
export const connection: ConnectionView = { serverUrl: 'https://lab:5241', serverFingerprint: fingerprint, clientId: 'bench-1', hasSecret: true, nextServerFingerprint: null };

/** The answers every page's requests get, unless a test routes its own. */
export const baseRoutes = {
  'GET /api/settings': () => json(settings),
  'GET /api/activity?max=200': () => json([]),
  'GET /api/server/identity': () => json({ name: 'bench-pc', fingerprint, httpsPort: 5241 }),
  'GET /api/system/checks': () => json([]),
  'GET /api/system/info': () =>
    json({ name: 'bench-pc', version: '1.0.0', startedAt: '2026-01-01T00:00:00Z', role: 'server', uiPort: 5240, httpsPort: 5241, fingerprint, os: 'test', tray: false }),
  'GET /api/client/discovery': () => json([]),
  'GET /api/client/pairing': () => json({ state: 'idle', serverName: null, serverUrl: null, code: null, message: null, expiresAt: null }),
  'GET /api/client/connection': () => json(connection),
  'GET /api/client/devices': () => json({ link: 'connected', linkMessage: null, devices: [], unavailable: null, gateway: false }),
  'GET /api/server/devices': () => json({ devices: [], unavailable: null }),
  'GET /api/server/clients': () => json([]),
  'GET /api/server/pairing': () => json({ budgetExhausted: false, window: { open: false, alwaysOpen: false, until: null }, requests: [] }),
};

export const frame = (overrides: Partial<FrameApi> = {}): FrameApi => ({ locked: false, asideSlot: null, ...overrides });

export const configuration = (locked = false) => (
  <FrameContext.Provider value={frame({ locked })}>
    <ConfigurationPage />
  </FrameContext.Provider>
);
