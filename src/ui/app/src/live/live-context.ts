import { createContext, useContext } from 'react';
/**
 * The UI's own link to Koppel, which is not a server's link: live while the push stream is open,
 * reconnecting while it retries, down once it has been away long enough that Koppel is probably not running.
 */
export type LiveState = 'live' | 'reconnecting' | 'down';

export interface LiveContextValue {
  state: LiveState;
  /** Opens the stream again at once, without waiting for the next retry step. */
  retry: () => void;
}

export const LiveContext = createContext<LiveContextValue | null>(null);

export function useLive(): LiveContextValue {
  const value = useContext(LiveContext);
  if (!value) throw new Error('useLive must be used inside LiveProvider.');
  return value;
}
