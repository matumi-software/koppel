import { getJson, rejection, sendChange } from './http';

/** Mirrors Koppel.Server.ClientView. */
export interface ClientView {
  id: string;
  name: string;
  createdAt: string;
  /** False for clients made before secrets were kept: rotate to get one to copy. */
  hasSecret: boolean;
  /** When it last got a token, since the server started. */
  lastSeen: string | null;
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
  const response = await fetch('/api/server/clients', {
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
  const response = await fetch(`/api/server/clients/${encodeURIComponent(clientId)}/secret`);
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? `Could not load the secret (${response.status}).`);
  }
  return ((await response.json()) as { clientSecret: string }).clientSecret;
}

/** Replaces the secret and returns the new one. The client list follows over the hub. */
export async function rotateClientSecret(clientId: string): Promise<string> {
  const response = await fetch(`/api/server/clients/${encodeURIComponent(clientId)}/rotate`, { method: 'POST' });
  if (!response.ok) throw await rejection(response);
  return ((await response.json()) as { clientSecret: string }).clientSecret;
}

export const revokeClient = (clientId: string) => sendChange(`/api/server/clients/${encodeURIComponent(clientId)}`, 'DELETE');
