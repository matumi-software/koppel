import { createContext, useContext, useEffect } from 'react';
import type { LinkState } from '@koppel/react-ui';

/** One server push: its JSON payload. */
export type PushHandler = (payload: unknown) => void;

/** The live push as the app sees it: subscribe to an event by its name. */
export interface PushSource {
  on: (name: string, handler: PushHandler) => void;
  off: (name: string, handler: PushHandler) => void;
}

export interface HubContextValue {
  connection: PushSource;
  state: LinkState;
  retry: () => void;
}

export const HubContext = createContext<HubContextValue | null>(null);

export function useHub(): HubContextValue {
  const value = useContext(HubContext);
  if (!value) throw new Error('useHub must be used inside HubProvider.');
  return value;
}

/** Subscribes to one server push for the lifetime of the component. */
export function useHubEvent<T>(name: string, handler: (payload: T) => void) {
  const { connection } = useHub();
  useEffect(() => {
    const listener = handler as PushHandler;
    connection.on(name, listener);
    return () => connection.off(name, listener);
  }, [connection, name, handler]);
}
