import { getJson, readJson, sendChange, uiFetch } from './http';
import type { ConnectionTest, ConnectionUpdate, ConnectionView } from './generated/wire';

export type { ConnectionTest, ConnectionUpdate, ConnectionView, Outcome } from './generated/wire';

export const getConnection = () => getJson<ConnectionView>('/api/client/connection');

/** `clientSecret` empty keeps the stored secret. */
export const putConnection = (update: ConnectionUpdate) => sendChange('/api/client/connection', 'PUT', update);

export async function testConnection(): Promise<ConnectionTest> {
  const response = await uiFetch('/api/client/connection/test', { method: 'POST' });
  if (!response.ok) {
    return { ok: false, outcome: 'unexpectedAnswer', message: `The test failed (${response.status}).` };
  }
  return readJson<ConnectionTest>(response);
}
