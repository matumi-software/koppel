import { getJson, readJson, rejection, sendChange, uiFetch } from './http';
import type { ClientView, CreateClient, EnrollmentKey, EnrollmentView, KeysView, NewClient, OpenEnrollmentCode, ServerIdentity } from './generated/wire';

export type { ClientView, EnrollmentCodeView, EnrollmentView, KeysView, NewClient, ServerIdentity } from './generated/wire';

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

export const getEnrollment = () => getJson<EnrollmentView>('/api/v1/server/enrollment');

/** How long a pairing code shows when a person asks for one, in seconds. The server holds it to between 30 and 300. */
export const enrollmentCodeSeconds = 60;

/** Shows a new pairing code (replacing one that shows) or closes it. Both answer 202; the code arrives as EnrollmentChanged. */
export const openEnrollmentCode = () => {
  const open: OpenEnrollmentCode = { seconds: enrollmentCodeSeconds };
  return sendChange('/api/v1/server/enrollment/code', 'PUT', open);
};
export const closeEnrollmentCode = () => sendChange('/api/v1/server/enrollment/code', 'DELETE');

/** The server key, fetched only when someone reveals or copies it (made the first time it is asked for). */
export async function getEnrollmentKey(): Promise<string> {
  // Through getJson, so a 401 shows the login screen.
  return (await getJson<EnrollmentKey>('/api/v1/server/enrollment/key')).key;
}

/** Replaces the server key and returns the new one, given only here. Clients already enrolled are not affected. */
export async function rotateEnrollmentKey(): Promise<string> {
  const response = await uiFetch('/api/v1/server/enrollment/key/rotation', { method: 'POST' });
  if (!response.ok) throw await rejection(response);
  return (await readJson<EnrollmentKey>(response)).key;
}
