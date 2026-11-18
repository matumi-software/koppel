import type { ClientServersView } from '@/api/generated/wire';
import { FrameContext, type FrameApi } from '@/shell/frame-context';
import { json } from './harness';
import { clientDevices, enrollmentView, pairingOverview, serverIdentity, servers, settings as settingsFixture, systemInfo } from './wire';

export const settings = settingsFixture;
export const fingerprint = serverIdentity.fingerprint;

const closedWindow = { open: false, alwaysOpen: false, until: null };

/** A client with no server linked: what a machine shows before anything is added. */
export const noServers: ClientServersView = { servers: [], unavailable: null, gateway: false, gatewayFault: null };

/** The answers every page's requests get, unless a test routes its own. */
export const baseRoutes = {
  'GET /api/v1/settings': () => json(settings),
  'GET /api/v1/activity?max=200': () => json([]),
  'GET /api/v1/server/identity': () => json(serverIdentity),
  'GET /api/v1/system/checks': () => json([]),
  'GET /api/v1/system/info': () => json(systemInfo),
  'GET /api/v1/client/discovery': () => json([]),
  'GET /api/v1/client/servers': () => json(servers),
  'GET /api/v1/client/devices': () => json({ ...clientDevices, ...noServers }),
  'GET /api/v1/server/devices': () => json({ devices: [], unavailable: null }),
  'GET /api/v1/server/clients': () => json([]),
  'GET /api/v1/server/enrollment': () => json({ ...enrollmentView, code: null }),
  'GET /api/v1/server/pairing': () => json({ ...pairingOverview, requests: [], window: closedWindow }),
};

export const frame = (overrides: Partial<FrameApi> = {}): FrameApi => ({ locked: false, narrow: false, ...overrides });

/** A page rendered inside the frame's context, for a test of one page. */
export const inFrame = (page: React.ReactNode, overrides: Partial<FrameApi> = {}) => <FrameContext.Provider value={frame(overrides)}>{page}</FrameContext.Provider>;
