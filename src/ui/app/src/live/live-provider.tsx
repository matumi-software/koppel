import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { useAnnounce, type LinkState } from '@koppel/react-ui';
import { useAuth } from '@/auth/auth-context';
import { LiveContext } from './live-context';
import { applyPush, downAfterFailures, eventsUrl, pushEvents, retryDelay } from './push-table';

/**
 * Owns the UI's live link to the server, one EventSource on /api/events, applies every push to the
 * query cache and reports the link's state. The browser would reconnect by itself, but not after a
 * refused request, so a failed stream is closed and a new one opened on retryDelay's steps, the first
 * connect included, asking the server each time whether the session is still good. Once the link is
 * back after a failure, everything is fetched again: what was pushed meanwhile is lost. The first
 * open needs no refetch, the queries just ran.
 */
export function LiveProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const { recheck } = useAuth();
  const announce = useAnnounce();
  const [state, setState] = useState<LinkState>('reconnecting');
  // Bumped by retry, which starts the link afresh.
  const [start, setStart] = useState(0);

  useEffect(() => {
    let source: EventSource | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let failures = 0;
    let stopped = false;

    const connect = () => {
      const current = new EventSource(eventsUrl);
      source = current;
      for (const name of pushEvents) {
        current.addEventListener(name, (event) => applyPush(queryClient, name, (event as MessageEvent<string>).data));
      }
      current.onopen = () => {
        if (stopped || source !== current) return;
        const away = failures > 0;
        failures = 0;
        setState('live');
        if (away) {
          announce('Live again. Changes can be saved.');
          void queryClient.invalidateQueries();
        }
      };
      current.onerror = () => {
        current.close();
        if (stopped || source !== current) return;
        // The server ends the stream when the session ends (sign-out, password change), and a refused
        // stream may be a session that ended during an outage: the gate shows the login screen if so.
        recheck();
        setState(failures >= downAfterFailures ? 'down' : 'reconnecting');
        // Said when the link goes away and when it is called down, not on every retry.
        if (failures === 0) announce('Reconnecting. Changes are paused until the link is back.');
        else if (failures === downAfterFailures) announce('Disconnected. Check that Koppel is running, then retry.');
        timer = setTimeout(connect, retryDelay(failures++));
      };
    };

    connect();
    return () => {
      stopped = true;
      clearTimeout(timer);
      source?.close();
    };
  }, [queryClient, recheck, announce, start]);

  const retry = useCallback(() => {
    setState('reconnecting');
    setStart((n) => n + 1);
  }, []);

  const value = useMemo(() => ({ state, retry }), [state, retry]);
  return <LiveContext.Provider value={value}>{children}</LiveContext.Provider>;
}
