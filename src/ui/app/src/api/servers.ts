import { getJson, readJson, rejection, sendChange, uiFetch } from './http';
import type { AddServer, ConnectionTest, ServerSettings, ServerView, ServersView } from './generated/wire';

export type { AddServer, ConnectionTest, Outcome, ServerSettings, ServerView, ServersView } from './generated/wire';

export const getServers = () => getJson<ServersView>('/api/v1/client/servers');

/**
 * Adds a server by its address and a pairing code or a server key. It answers 201 with the saved
 * server, not 202, because the person has to read why a server refused: the thrown RejectedError
 * carries the problem (`problem.code` is `enrolment_failed` or `already_added`, `problem.errors` is
 * the message per field of a 400). The list itself follows as ServersChanged.
 */
export async function addServer(add: AddServer): Promise<ServerView> {
  const response = await uiFetch('/api/v1/client/servers', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(add),
  });
  if (response.status !== 201) throw await rejection(response);
  return readJson<ServerView>(response);
}

/** Answers 202; the change shows in ServersChanged once the server wrote it. A link setting makes the link again. */
export const changeServer = (id: string, change: ServerSettings) => sendChange(`/api/v1/client/servers/${encodeURIComponent(id)}`, 'PATCH', change);

/** Releases the server's devices and forgets it. Answers 202. */
export const removeServer = (id: string) => sendChange(`/api/v1/client/servers/${encodeURIComponent(id)}`, 'DELETE');

/** A query, so it answers with its result. */
export async function testServer(id: string): Promise<ConnectionTest> {
  const response = await uiFetch(`/api/v1/client/servers/${encodeURIComponent(id)}/test`, { method: 'POST' });
  if (!response.ok) {
    return { ok: false, outcome: 'unexpectedAnswer', message: `The test failed (${response.status}).` };
  }
  return readJson<ConnectionTest>(response);
}
