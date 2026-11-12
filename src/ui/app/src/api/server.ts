import { getJson, readJson, rejection, sendChange, uiFetch } from './http';
import type { ClientView, CreateClient, KeysView, NewClient, ServerIdentity } from './generated/wire';

export type { ClientView, KeysView, NewClient, ServerIdentity } from './generated/wire';

export const getIdentity = () => getJson<ServerIdentity>('/api/v1/server/identity');

export const getClients = () => getJson<ClientView[]>('/api/v1/server/clients');

/** Creates a client. The list itself updates when ClientsChanged arrives. */
export async function createClient(name: string): Promise<NewClient> {
  const body: CreateClient = { name };
  const response = await uiFetch('/api/v1/server/clients', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (response.status !== 201) {
    throw await rejection(response);
  }
  return readJson<NewClient>(response);
}

/** One client's secret, fetched only when someone reveals or copies it. */
export async function getClientSecret(clientId: string): Promise<string> {
  // Through getJson, so a 401 shows the login screen.
  return (await getJson<{ clientSecret: string }>(`/api/v1/server/clients/${encodeURIComponent(clientId)}/secret`)).clientSecret;
}

/** Replaces the secret and returns the new one. The client list follows over the live push. */
export async function rotateClientSecret(clientId: string): Promise<string> {
  const response = await uiFetch(`/api/v1/server/clients/${encodeURIComponent(clientId)}/secret`, { method: 'POST' });
  if (!response.ok) throw await rejection(response);
  return (await readJson<{ clientSecret: string }>(response)).clientSecret;
}

export const getServerKeys = () => getJson<KeysView>('/api/v1/server/keys');

/** Each of these answers 202; the new state arrives as ServerKeysChanged. */
export const prepareCertificate = () => sendChange('/api/v1/server/keys/next-certificate', 'POST');
export const cancelCertificate = () => sendChange('/api/v1/server/keys/next-certificate', 'DELETE');
export const switchCertificate = () => sendChange('/api/v1/server/keys/current-certificate', 'PUT');

export const revokeClient = (clientId: string) => sendChange(`/api/v1/server/clients/${encodeURIComponent(clientId)}`, 'DELETE');
