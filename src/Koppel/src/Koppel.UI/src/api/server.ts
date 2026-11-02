import { getJson, rejection, sendChange } from './http';

/** Mirrors Koppel.Server.ClientView. */
export interface ClientView {
  id: string;
  name: string;
  createdAt: string;
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

export const revokeClient = (clientId: string) => sendChange(`/api/server/clients/${encodeURIComponent(clientId)}`, 'DELETE');
