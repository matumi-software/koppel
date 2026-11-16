import { useQuery } from '@tanstack/react-query';
import type { ClientView } from '@/api/server';
import { clientsOptions } from '@/queries';

/** The clients registered on this server (server role). ClientsChanged keeps them current. */
export function useClients(enabled = true) {
  return useQuery({ ...clientsOptions(), enabled });
}

/** A client's name by its id, the id itself when it is not in the list (yet). */
export const clientName = (clients: readonly ClientView[] | undefined, id: string | null | undefined) =>
  clients?.find((c) => c.id === id)?.name ?? id ?? 'a client';
