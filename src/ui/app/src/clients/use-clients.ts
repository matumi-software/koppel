import { useQuery } from '@tanstack/react-query';
import type { ClientView } from '@/api/server';
import { useServerDevices } from '@/devices/use-devices';
import { usePending } from '@/pending/pending-context';
import { clientsOptions } from '@/queries';

/**
 * The clients registered on this server (server role). ClientsChanged keeps them current, except for when a
 * client was last seen: Skakel does not push that (a client is seen at every token), so a page that shows it
 * asks again when it opens, and never shows a connected client as unseen from a list that is older than the page.
 */
export function useClients(enabled = true) {
  return useQuery({ ...clientsOptions(), enabled, refetchOnMount: 'always' });
}

/** A client's name by its id, the id itself when it is not in the list (yet). */
export const clientName = (clients: readonly ClientView[] | undefined, id: string | null | undefined) =>
  clients?.find((c) => c.id === id)?.name ?? id ?? 'a client';

/** The names of the devices a client has attached on this machine. */
export const useUsing = (clientId: string) => (useServerDevices().data?.devices ?? []).filter((d) => d.state === 'attached' && d.attachedTo === clientId).map((d) => d.name);

/** Revoking a client under the confirm rule: asked first, pending until the list no longer has it. */
export function useRevoke() {
  const { pending, failures, request } = usePending('clients');
  return { pending, failures, revoke: (client: Pick<ClientView, 'id' | 'name'>) => request({ id: client.id, name: client.name }) };
}
