import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useState, type ReactNode } from 'react';
import type { LinkState } from '@koppel/react-ui';
import { useAuth } from '@/auth/auth-context';
import { HubContext, type PushHandler, type PushSource } from './hub-context';
import { eventsUrl, pushEvents, retryDelay } from './push-events';

/** Hands each event to the handlers registered for its name. One for the provider's life, across reconnects. */
class Dispatcher implements PushSource {
  private readonly handlers = new Map<string, Set<PushHandler>>();

  on = (name: string, handler: PushHandler) => {
    const set = this.handlers.get(name) ?? new Set();
    set.add(handler);
    this.handlers.set(name, set);
  };

  off = (name: string, handler: PushHandler) => {
    this.handlers.get(name)?.delete(handler);
  };

  dispatch(name: string, data: string) {
    const handlers = this.handlers.get(name);
    if (!handlers?.size) return;
    let payload: unknown;
    try {
      payload = JSON.parse(data);
    } catch {
      return;
    }
    for (const handler of [...handlers]) handler(payload);
  }
}

/**
 * Owns the UI's live link to the server, an EventSource on /api/events, and reports its state for
 * LinkBanner. The browser would reconnect by itself, but not after a refused request, so the provider
 * closes a failed stream and opens a new one on its own steps, asking the server each time whether the
 * session is still good. Once open, everything is fetched again: what was pushed meanwhile is lost.
 */
export function HubProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const { recheck } = useAuth();
  const [connection] = useState(() => new Dispatcher());
  const [state, setState] = useState<LinkState>('reconnecting');
  // Bumped by the retry button, which starts the link afresh.
  const [start, setStart] = useState(0);

  useEffect(() => {
    let source: EventSource | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let attempt = 0;
    let wasLive = false;
    let stopped = false;

    const connect = () => {
      const current = new EventSource(eventsUrl);
      source = current;
      for (const name of pushEvents) {
        current.addEventListener(name, (event) => connection.dispatch(name, (event as MessageEvent<string>).data));
      }
      current.onopen = () => {
        if (stopped || source !== current) return;
        wasLive = true;
        attempt = 0;
        setState('live');
        // Anything pushed while we were away is lost; refetch before trusting the cache.
        void queryClient.invalidateQueries();
      };
      current.onerror = () => {
        current.close();
        if (stopped || source !== current) return;
        // The server ends the stream when the session ends (sign-out, password change), and a refused
        // stream may be a session that ended during an outage: the gate shows the login screen if so.
        recheck();
        if (!wasLive) {
          // The first start failed: the person retries, as with any link that never came up.
          setState('down');
          return;
        }
        setState('reconnecting');
        timer = setTimeout(connect, retryDelay(attempt++));
      };
    };

    connect();
    return () => {
      stopped = true;
      clearTimeout(timer);
      source?.close();
    };
  }, [connection, queryClient, recheck, start]);

  const retry = () => {
    setState('reconnecting');
    setStart((n) => n + 1);
  };

  return <HubContext.Provider value={{ connection, state, retry }}>{children}</HubContext.Provider>;
}
