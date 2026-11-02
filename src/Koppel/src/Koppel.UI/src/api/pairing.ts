import { getJson, sendChange } from './http';

/** Mirrors Koppel.Core.Discovery.DiscoveredServer: a server found on the local network. */
export interface DiscoveredServer {
  name: string;
  address: string;
  httpsPort: number;
  fingerprint: string;
  version: string | null;
  url: string;
}

/** `confirm`: the server approved, but saving replaces the connection to another server, so the person decides. */
export type PairingState = 'idle' | 'requesting' | 'waiting' | 'confirm' | 'paired' | 'denied' | 'expired' | 'failed';

/** Mirrors Koppel.Client.PairingStatus. While waiting, `code` is what to type on the server. */
export interface PairingStatus {
  state: PairingState;
  serverName: string | null;
  serverUrl: string | null;
  code: string | null;
  message: string | null;
  expiresAt: string | null;
  serverFingerprint: string | null;
}

/** Mirrors Koppel.Server.PairingRequestView. The code is not in it: a person types it. `denied` shows for a moment after three wrong codes. */
export interface PairingRequest {
  id: string;
  clientName: string;
  from: string;
  requestedAt: string;
  expiresAt: string;
  state: 'waiting' | 'denied';
  message: string | null;
}

export const getDiscoveredServers = () => getJson<DiscoveredServer[]>('/api/client/discovery');

export const getPairingStatus = () => getJson<PairingStatus>('/api/client/pairing');

export const startPairing = (server: DiscoveredServer) =>
  sendChange('/api/client/pairing', 'POST', { serverUrl: server.url, serverFingerprint: server.fingerprint, serverName: server.name });

/** Saves the credentials the server approved, replacing the connection to another server. */
export const confirmPairing = () => sendChange('/api/client/pairing/confirm', 'POST');

export const cancelPairing = () => sendChange('/api/client/pairing', 'DELETE');

/** Mirrors Koppel.Server.PairingOverview. `budgetExhausted`: the server takes no new pairing requests until its window moves on. */
export interface PairingOverview {
  requests: PairingRequest[];
  budgetExhausted: boolean;
}

export const getPairingRequests = () => getJson<PairingOverview>('/api/server/pairing');

export const approvePairing = (id: string, code: string) => sendChange(`/api/server/pairing/${encodeURIComponent(id)}/approve`, 'POST', { code });

export const denyPairing = (id: string) => sendChange(`/api/server/pairing/${encodeURIComponent(id)}/deny`, 'POST');
