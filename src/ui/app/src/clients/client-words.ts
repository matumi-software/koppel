import type { ClientView } from '@/api/server';

/** How a client was linked, in words: pairing code, server key, a pairing request, or by hand. */
export const linkedWord = (client: Pick<ClientView, 'linkedWith'>) =>
  client.linkedWith === 'code' ? 'Pairing code' : client.linkedWith === 'key' ? 'Server key' : client.linkedWith === 'pairing' ? 'Pairing request' : 'Added by hand';

const clock = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });
const date = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' });

/** The day a client was added. */
export const addedOn = (client: Pick<ClientView, 'createdAt'>) => date.format(new Date(client.createdAt));

/** When a client last called, in words that move on with the clock passed in (see useNow). */
export const lastSeen = (client: Pick<ClientView, 'lastSeen'>, now: number) => {
  if (!client.lastSeen) return 'Not since Koppel started';
  const minutes = Math.round((now - new Date(client.lastSeen).getTime()) / 60_000);
  return minutes < 1 ? 'Just now' : minutes < 60 ? `${minutes} min ago` : `At ${clock.format(new Date(client.lastSeen))}`;
};
