import { formatRelative } from '@heartwood/ui';
import type { ClientView } from '@/api/server';
import { date } from '@/shared/time';

/** How a client was linked, in words: pairing code, server key, a pairing request, or by hand. */
export const linkedWord = (client: Pick<ClientView, 'linkedWith'>) =>
  client.linkedWith === 'code' ? 'Pairing code' : client.linkedWith === 'key' ? 'Server key' : client.linkedWith === 'pairing' ? 'Pairing request' : 'Added by hand';


/** The day a client was added. */
export const addedOn = (client: Pick<ClientView, 'createdAt'>) => date.format(new Date(client.createdAt));

/** A sentence for a client's row: "Last seen 5 min ago", or that it has not been seen since Koppel started. */
export const lastSeenLine = (client: Pick<ClientView, 'lastSeen'>, now: number) =>
  client.lastSeen ? `Last seen ${formatRelative(client.lastSeen, { now })}` : 'Not seen since Koppel started';

/** When a client last called, in Heartwood's words ("Just now", "5 min ago", "Yesterday"), moving on with the clock passed in (see useNow). */
export const lastSeen = (client: Pick<ClientView, 'lastSeen'>, now: number) => {
  if (!client.lastSeen) return 'Not since Koppel started';
  const when = formatRelative(client.lastSeen, { now });
  return `${when.charAt(0).toUpperCase()}${when.slice(1)}`;
};
