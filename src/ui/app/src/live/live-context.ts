import { createContext, useContext } from 'react';
import type { LinkState } from '@koppel/react-ui';

export interface LiveContextValue {
  state: LinkState;
  /** Opens the stream again at once, without waiting for the next retry step. */
  retry: () => void;
}

export const LiveContext = createContext<LiveContextValue | null>(null);

export function useLive(): LiveContextValue {
  const value = useContext(LiveContext);
  if (!value) throw new Error('useLive must be used inside LiveProvider.');
  return value;
}
