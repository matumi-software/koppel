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

export const getDiscoveredServers = () => getJson<DiscoveredServer[]>('/api/v1/client/discovery');

export const getPairingStatus = () => getJson<PairingStatus>('/api/v1/client/pairing');

export const startPairing = (server: DiscoveredServer) => {
  const start: PairStart = { serverUrl: server.url, serverFingerprint: server.fingerprint, serverName: server.name };
  return sendChange('/api/v1/client/pairing', 'POST', start);
};

/** Saves the credentials the server approved, replacing the connection to another server. */
export const confirmPairing = () => sendChange('/api/v1/client/pairing/confirmation', 'POST');

export const cancelPairing = () => sendChange('/api/v1/client/pairing', 'DELETE');

/** Both answer 202; the window shows as open or closed when PairingRequestsChanged arrives. */
export const openPairingWindow = () => {
  const open: OpenPairing = { minutes: pairingWindowMinutes };
  return sendChange('/api/v1/server/pairing/window', 'PUT', open);
};
export const closePairingWindow = () => sendChange('/api/v1/server/pairing/window', 'DELETE');

export const getPairingRequests = () => getJson<PairingOverview>('/api/v1/server/pairing');

export const approvePairing = (id: string, code: string) => sendChange(`/api/v1/server/pairing/requests/${encodeURIComponent(id)}/approval`, 'POST', { code });

export const denyPairing = (id: string) => sendChange(`/api/v1/server/pairing/requests/${encodeURIComponent(id)}`, 'DELETE');
