import type { HubConnection } from '@microsoft/signalr';
import { createContext, useContext, useEffect } from 'react';
import type { LinkState } from '@koppel/react-ui';

export interface HubContextValue {
  connection: HubConnection;
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
    connection.on(name, handler);
    return () => connection.off(name, handler);
  }, [connection, name, handler]);
}
