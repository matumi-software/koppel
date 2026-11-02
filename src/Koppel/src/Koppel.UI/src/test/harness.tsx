import type { HubConnection } from '@microsoft/signalr';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import type { ReactElement } from 'react';
import { vi } from 'vitest';
import { ActivityProvider } from '@/activity/activity-provider';
import { HubContext } from '@/hub/hub-context';
import { HubPushes } from '@/hub/hub-pushes';
import { PendingProvider } from '@/pending/pending-provider';

/** A HubConnection that only knows how to subscribe, and lets a test push. */
export class FakeHub {
  private readonly handlers = new Map<string, Set<(payload: unknown) => void>>();

  on = (name: string, handler: (payload: unknown) => void) => {
    const set = this.handlers.get(name) ?? new Set();
    set.add(handler);
    this.handlers.set(name, set);
  };

  off = (name: string, handler: (payload: unknown) => void) => {
    this.handlers.get(name)?.delete(handler);
  };

  /** How many handlers listen to an event. */
  count = (name: string) => this.handlers.get(name)?.size ?? 0;

  emit = (name: string, payload: unknown) => {
    for (const handler of [...(this.handlers.get(name) ?? [])]) handler(payload);
  };
}

export const newQueryClient = () =>
  new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, refetchOnWindowFocus: false, retry: false, gcTime: Infinity } } });

/** Renders under the same providers as main.tsx, with a fake hub. */
export function renderApp(ui: ReactElement, hub = new FakeHub(), queryClient = newQueryClient()) {
  const tree = (node: ReactElement) => (
    <QueryClientProvider client={queryClient}>
      <HubContext.Provider value={{ connection: hub as unknown as HubConnection, state: 'live', retry: () => {} }}>
        <HubPushes />
        <ActivityProvider>
          <PendingProvider>{node}</PendingProvider>
        </ActivityProvider>
      </HubContext.Provider>
    </QueryClientProvider>
  );
  const result = render(tree(ui));
  return { ...result, hub, queryClient, rerenderApp: (next: ReactElement) => result.rerender(tree(next)) };
}

export const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
export const accepted = () => new Response(null, { status: 202 });

type Route = (init?: RequestInit) => Response | Promise<Response>;

/** Replaces fetch with routes keyed "METHOD /url". An unrouted request fails the test loudly. */
export function mockFetch(routes: Record<string, Route>) {
  const calls: { key: string; body?: unknown }[] = [];
  const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    const key = `${init?.method ?? 'GET'} ${String(input)}`;
    calls.push({ key, body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined });
    const route = routes[key];
    if (!route) return Promise.reject(new Error(`Unrouted request: ${key}`));
    return Promise.resolve(route(init));
  });
  vi.stubGlobal('fetch', fetchMock);
  return { calls, fetchMock, count: (key: string) => calls.filter((c) => c.key === key).length };
}

/** A promise a test settles by hand, for a request that must stay in flight. */
export function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}
