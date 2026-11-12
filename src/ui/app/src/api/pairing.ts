import { getJson, sendChange } from './http';

/**
 * Mirrors Koppel.Discovery.DiscoveredServer: a server found on the local network. `otherAddresses`: other
 * addresses that answered with the same fingerprint.
 */
export interface DiscoveredServer {
  name: string;
  address: string;
  httpsPort: number;
  fingerprint: string;
  version: string | null;
  url: string;
  otherAddresses: string[];
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

/** Mirrors Skakel.Server.SkakelPairingRequest. The code is not in it: a person types it. `denied` shows for a moment after three wrong codes. */
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

/** Whether the server takes new pairing requests. It takes them only while a person has opened the window. */
export interface PairingWindow {
  open: boolean;
  /** The server takes requests at any time, so there is nothing to open. */
  alwaysOpen: boolean;
  /** When an open window closes by itself, or null. */
  until: string | null;
}

/** The pairing window and the requests waiting. `budgetExhausted`: the server takes no new pairing requests until its limit moves on. */
export interface PairingOverview {
  requests: PairingRequest[];
  budgetExhausted: boolean;
  window: PairingWindow;
}

/** How long the window stays open when a person opens it, in minutes. The server holds it to between 1 and 60. */
export const pairingWindowMinutes = 10;

/** Both answer 202; the window shows as open or closed when PairingRequestsChanged arrives. */
export const openPairingWindow = () => sendChange('/api/server/pairing/window', 'POST', { minutes: pairingWindowMinutes });
export const closePairingWindow = () => sendChange('/api/server/pairing/window', 'DELETE');

export const getPairingRequests = () => getJson<PairingOverview>('/api/server/pairing');

export const approvePairing = (id: string, code: string) => sendChange(`/api/server/pairing/${encodeURIComponent(id)}/approve`, 'POST', { code });

export const denyPairing = (id: string) => sendChange(`/api/server/pairing/${encodeURIComponent(id)}/deny`, 'POST');
