import { ConfigurationPage } from '@/settings/configuration-page';
import { FrameContext, type FrameApi } from '@/shell/frame-context';
import { json } from './harness';
import { clientDevices, connectionView, pairingOverview, pairingStatus, serverIdentity, settings as settingsFixture, systemInfo } from './wire';

export const settings = settingsFixture;
export const fingerprint = serverIdentity.fingerprint;
export const connection = connectionView;

const closedWindow = { open: false, alwaysOpen: false, until: null };

/** The answers every page's requests get, unless a test routes its own. */
export const baseRoutes = {
  'GET /api/settings': () => json(settings),
  'GET /api/activity?max=200': () => json([]),
  'GET /api/server/identity': () => json(serverIdentity),
  'GET /api/system/checks': () => json([]),
  'GET /api/system/info': () => json(systemInfo),
  'GET /api/client/discovery': () => json([]),
  'GET /api/client/pairing': () => json({ ...pairingStatus, state: 'idle', serverName: null, serverUrl: null, code: null, expiresAt: null, serverFingerprint: null }),
  'GET /api/client/connection': () => json(connection),
  'GET /api/client/devices': () => json({ ...clientDevices, devices: [], gateway: false, gatewayFault: null }),
  'GET /api/server/devices': () => json({ devices: [], unavailable: null }),
  'GET /api/server/clients': () => json([]),
  'GET /api/server/pairing': () => json({ ...pairingOverview, requests: [], window: closedWindow }),
};

export const frame = (overrides: Partial<FrameApi> = {}): FrameApi => ({ locked: false, asideSlot: null, hasNavigated: false, ...overrides });

export const configuration = (locked = false) => (
  <FrameContext.Provider value={frame({ locked })}>
    <ConfigurationPage />
  </FrameContext.Provider>
);
