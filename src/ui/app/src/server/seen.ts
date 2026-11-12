import type { ClientView } from '@/api/server';

/** When a client last called, in words that move on with the clock passed in (see useNow). */
export const seen = (client: ClientView, now: number) => {
  if (!client.lastSeen) return 'Not seen since the server started';
  const minutes = Math.round((now - new Date(client.lastSeen).getTime()) / 60_000);
  return minutes < 1 ? 'Seen just now' : minutes < 60 ? `Seen ${minutes} min ago` : `Seen at ${new Date(client.lastSeen).toLocaleTimeString()}`;
};
