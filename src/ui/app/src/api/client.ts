import { getJson, sendChange, uiFetch } from './http';

/** Mirrors Koppel.Client.ConnectionView. The secret never comes back, only whether one is stored. */
export interface ConnectionView {
  serverUrl: string | null;
  serverFingerprint: string | null;
  clientId: string | null;
  hasSecret: boolean;
  /** The certificate the server said it will switch to, trusted as well until then. */
  nextServerFingerprint: string | null;
}

export interface ConnectionUpdate {
  serverUrl: string;
  serverFingerprint: string;
  clientId: string;
  /** Empty keeps the stored secret. */
  clientSecret: string;
}

export interface ConnectionTestResult {
  ok: boolean;
  /** One of a fixed set, such as `unreachable`, `certificateMismatch` or `versionMismatch`. */
  outcome?: string;
  message: string;
}

export const getConnection = () => getJson<ConnectionView>('/api/client/connection');

export const putConnection = (update: ConnectionUpdate) => sendChange('/api/client/connection', 'PUT', update);

export async function testConnection(): Promise<ConnectionTestResult> {
  const response = await uiFetch('/api/client/connection/test', { method: 'POST' });
  if (!response.ok) {
    return { ok: false, outcome: 'failed', message: `The test failed (${response.status}).` };
  }
  return (await response.json()) as ConnectionTestResult;
}
