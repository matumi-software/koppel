import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { AnnouncerProvider } from '@koppel/react-ui';
import { render, screen } from '@testing-library/react';
import type { ReactElement } from 'react';
import { vi } from 'vitest';
import { ActivityProvider } from '@/activity/activity-provider';
import { AuthContext } from '@/auth/auth-context';
import { LiveProvider } from '@/live/live-provider';
import { PendingProvider } from '@/pending/pending-provider';
import { FakeEventSource } from './fake-event-source';

export const newQueryClient = () =>
  new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, refetchOnWindowFocus: false, retry: false, gcTime: Infinity } } });

const signedIn = { recheck: () => {}, signOut: async () => {} };

/** What a test does to the live link the app opened: push an event, or count who listens to it. */
export const live = {
  emit: (name: string, payload: unknown) => FakeEventSource.emit(name, payload),
  emitRaw: (name: string, data: string) => FakeEventSource.current.emitRaw(name, data),
  count: (name: string) => FakeEventSource.current.count(name),
};

/** Renders under the same providers as main.tsx, with a fake EventSource behind the live link. */
export function renderApp(ui: ReactElement, queryClient = newQueryClient()) {
  const tree = (node: ReactElement) => (
    <QueryClientProvider client={queryClient}>
      <AnnouncerProvider>
        <AuthContext.Provider value={signedIn}>
          <LiveProvider>
            <ActivityProvider>
              <PendingProvider>{node}</PendingProvider>
            </ActivityProvider>
          </LiveProvider>
        </AuthContext.Provider>
      </AnnouncerProvider>
    </QueryClientProvider>
  );
  const result = render(tree(ui));
  return { ...result, live, queryClient, rerenderApp: (next: ReactElement) => result.rerender(tree(next)) };
}

/** What the app's one live region says now: what a screen reader was last told. */
export const announced = () => screen.getByRole('status').textContent;

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
