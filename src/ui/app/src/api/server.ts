import { getJson, rejection, sendChange, uiFetch } from './http';

/** Mirrors Skakel.Server.SkakelClientView, as the server pushes it. */
export interface ClientView {
  id: string;
  name: string;
  createdAt: string;
  /** False for clients made before secrets were kept: rotate to get one to copy. */
  hasSecret: boolean;
  /** When it last got a token, since the server started. */
  lastSeen: string | null;
  /** It stored the fingerprint of the prepared next certificate. */
  hasNextCertificate: boolean;
}

/** Mirrors Skakel.Server.SkakelKeysView: the certificate and the token signing key. */
export interface ServerKeysView {
  fingerprint: string;
  /** The prepared next certificate, or null when none is prepared. */
  nextFingerprint: string | null;
  nextPreparedAt: string | null;
  signingKeyId: string;
  /** When the signing key was made. */
  signingKeySince: string;
  /** While replaced keys are still accepted, when the last of them is dropped. */
  retiredKeysUntil: string | null;
}

export interface ServerIdentity {
  name: string;
  fingerprint: string;
  httpsPort: number;
}

/** Returned once when a client is created. The secret is never shown again. */
export interface CreatedClient {
  clientId: string;
  clientSecret: string;
  name: string;
}

export const getIdentity = () => getJson<ServerIdentity>('/api/server/identity');

export const getClients = () => getJson<ClientView[]>('/api/server/clients');

/** Creates a client. The list itself updates when ClientsChanged arrives. */
export async function createClient(name: string): Promise<CreatedClient> {
  const response = await uiFetch('/api/server/clients', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name }),
  });
  if (response.status !== 201) {
    throw await rejection(response);
  }
  return (await response.json()) as CreatedClient;
}

/** One client's secret, fetched only when someone reveals or copies it. */
export async function getClientSecret(clientId: string): Promise<string> {
  // Through getJson, so a 401 shows the login screen.
  return (await getJson<{ clientSecret: string }>(`/api/server/clients/${encodeURIComponent(clientId)}/secret`)).clientSecret;
}

/** Replaces the secret and returns the new one. The client list follows over the live push. */
export async function rotateClientSecret(clientId: string): Promise<string> {
  const response = await uiFetch(`/api/server/clients/${encodeURIComponent(clientId)}/rotate`, { method: 'POST' });
  if (!response.ok) throw await rejection(response);
  return ((await response.json()) as { clientSecret: string }).clientSecret;
}

export const getServerKeys = () => getJson<ServerKeysView>('/api/server/keys');

/** Each of these answers 202; the new state arrives as ServerKeysChanged. */
export const rotateSigningKey = () => sendChange('/api/server/keys/signing-key/rotate', 'POST');
export const prepareCertificate = () => sendChange('/api/server/keys/next-certificate', 'POST');
export const cancelCertificate = () => sendChange('/api/server/keys/next-certificate', 'DELETE');
export const switchCertificate = () => sendChange('/api/server/keys/next-certificate/switch', 'POST');

export const revokeClient = (clientId: string) => sendChange(`/api/server/clients/${encodeURIComponent(clientId)}`, 'DELETE');
