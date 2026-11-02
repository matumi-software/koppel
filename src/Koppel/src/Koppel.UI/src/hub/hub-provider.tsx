import { HubConnectionBuilder, HubConnectionState } from '@microsoft/signalr';
import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { LinkState } from '@koppel/react-ui';
import { useAuth } from '@/auth/auth-context';
import { HubContext } from './hub-context';

const retryDelays = [0, 2000, 5000, 10000, 30000];

function createConnection() {
  return new HubConnectionBuilder()
    .withUrl('/api/hubs/ui')
    // Keep retrying forever; the last delay repeats.
    .withAutomaticReconnect({
      nextRetryDelayInMilliseconds: (context) => retryDelays[Math.min(context.previousRetryCount, retryDelays.length - 1)],
    })
    .build();
}

/** Owns the UI's live link to the server and reports its state for LinkBanner. */
export function HubProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const { recheck } = useAuth();
  const [connection] = useState(createConnection);
  const [state, setState] = useState<LinkState>('reconnecting');
  // A start must wait for the previous stop (StrictMode mounts, stops and mounts again).
  const stopping = useRef<Promise<void>>(Promise.resolve());
  const active = useRef(false);

  // State changes only in promise callbacks, never synchronously.
  const start = () =>
    stopping.current
      .then(() => {
        if (connection.state === HubConnectionState.Disconnected) return connection.start();
      })
      .then(
        () => {
          if (!active.current) return;
          setState('live');
          // Anything pushed while we were away is lost; refetch before trusting the cache.
          void queryClient.invalidateQueries();
        },
        () => {
          if (active.current) setState('down');
          // A refused start may be a session that ended; the gate shows the login screen if so.
          recheck();
        },
      );

  useEffect(() => {
    active.current = true;
    connection.onreconnecting(() => {
      setState('reconnecting');
      recheck();
    });
    connection.onreconnected(() => {
      setState('live');
      void queryClient.invalidateQueries();
    });
    connection.onclose(() => {
      if (active.current) setState('down');
      // The server closes the hub when the session ends (sign-out, password change).
      recheck();
    });
    void start();
    return () => {
      active.current = false;
      stopping.current = connection.stop();
    };
    // start only reads refs and stable values.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connection, queryClient, recheck]);

  const retry = () => {
    setState('reconnecting');
    void start();
  };

  return <HubContext.Provider value={{ connection, state, retry }}>{children}</HubContext.Provider>;
}
