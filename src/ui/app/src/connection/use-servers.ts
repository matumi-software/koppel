import { useMutation, useQuery } from '@tanstack/react-query';
import { useCallback } from 'react';
import { addServer, type ServerView } from '@/api/servers';
import type { ServerSettings } from '@/api/servers';
import { usePending } from '@/pending/pending-context';
import { serversOptions } from '@/queries';

/** The servers this machine attaches from (client role). ServersChanged keeps them current. */
export function useServers(enabled = true) {
  return useQuery({ ...serversOptions(), enabled });
}

/** A server's name as shown: the one it gave itself, else its address. */
export const serverLabel = (server: Pick<ServerView, 'name' | 'serverUrl'>) => server.name ?? server.serverUrl ?? 'Unnamed server';

/**
 * Changing or removing a server under the confirm rule. `pending` and `failures` are by server id;
 * a change is done when ServersChanged shows it. `change` sends only the fields it is given.
 */
export function useServerChanges() {
  const { pending, failures, saved, request } = usePending('servers');
  const change = useCallback(
    (server: ServerView, fields: ServerSettings) => request({ type: 'change', id: server.id, name: serverLabel(server), change: fields }),
    [request],
  );
  const remove = useCallback((server: ServerView) => request({ type: 'remove', id: server.id, name: serverLabel(server) }), [request]);
  return { pending, failures, saved, change, remove };
}

/**
 * Add a server by address and a pairing code or a server key. Its 201 carries the saved server, and a
 * refusal carries the reason (`error` is a RejectedError: `error.message` is the first sentence,
 * `error.problem?.code` is `enrolment_failed` or `already_added`, `error.problem?.errors` the message per field of a 400).
 * The list follows as ServersChanged.
 */
export function useAddServer() {
  return useMutation({ mutationFn: addServer });
}
