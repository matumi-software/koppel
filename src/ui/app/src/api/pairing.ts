import { getJson, sendChange } from './http';
import type { DiscoveredServer, OpenPairing, PairingOverview, PairingStatus, PairStart } from './generated/wire';

export type {
  DiscoveredServer,
  PairingOverview,
  PairingRequestView,
  PairingState,
  PairingStatus,
  PairingWindowView,
  RequestState,
} from './generated/wire';

/** How long the window stays open when a person opens it, in minutes. The server holds it to between 1 and 60. */
export const pairingWindowMinutes = 10;

export const getDiscoveredServers = () => getJson<DiscoveredServer[]>('/api/client/discovery');

export const getPairingStatus = () => getJson<PairingStatus>('/api/client/pairing');

export const startPairing = (server: DiscoveredServer) => {
  const start: PairStart = { serverUrl: server.url, serverFingerprint: server.fingerprint, serverName: server.name };
  return sendChange('/api/client/pairing', 'POST', start);
};

/** Saves the credentials the server approved, replacing the connection to another server. */
export const confirmPairing = () => sendChange('/api/client/pairing/confirm', 'POST');

export const cancelPairing = () => sendChange('/api/client/pairing', 'DELETE');

/** Both answer 202; the window shows as open or closed when PairingRequestsChanged arrives. */
export const openPairingWindow = () => {
  const open: OpenPairing = { minutes: pairingWindowMinutes };
  return sendChange('/api/server/pairing/window', 'POST', open);
};
export const closePairingWindow = () => sendChange('/api/server/pairing/window', 'DELETE');

export const getPairingRequests = () => getJson<PairingOverview>('/api/server/pairing');

export const approvePairing = (id: string, code: string) => sendChange(`/api/server/pairing/${encodeURIComponent(id)}/approve`, 'POST', { code });

export const denyPairing = (id: string) => sendChange(`/api/server/pairing/${encodeURIComponent(id)}/deny`, 'POST');
