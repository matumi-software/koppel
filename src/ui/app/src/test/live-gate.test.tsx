import type { AuthState } from '@/api/auth';
import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { getJson } from '@/api/http';
import { eventsUrl } from '@/live/push-table';
import { FakeEventSource } from './fake-event-source';
import { deferred, json, mockFetch } from './harness';
import { loginHeading, noContent, renderGate, state } from './auth-fixtures';

afterEach(() => vi.unstubAllGlobals());

const live = () => FakeEventSource.live;

describe('the live push under the sign-in gate', () => {
  // Signed in unless a test says otherwise.
  const inState = (overrides: Partial<AuthState> = {}) => state({ signedIn: true, ...overrides });

  it('starts when the browser signs in, and not before', async () => {
    let signedIn = false;
    mockFetch({ 'GET /api/auth/state': () => json(inState({ signedIn })), 'POST /api/auth/login': noContent });
    renderGate();
    await userEvent.type(await screen.findByLabelText('Password'), 'correct horse battery');
    expect(FakeEventSource.instances.length).toBe(0);

    signedIn = true;
    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));

    await screen.findByText('The app');
    await waitFor(() => expect(live().length).toBe(1));
    expect(live()[0].url).toBe(eventsUrl);
  });

  it('stops on sign-out and shows the login screen', async () => {
    let signedIn = true;
    const { calls } = mockFetch({
      'GET /api/auth/state': () => json(inState({ signedIn })),
      'POST /api/auth/logout': () => {
        signedIn = false;
        return noContent();
      },
    });
    renderGate();
    await screen.findByText('The app');
    await waitFor(() => expect(live().length).toBe(1));

    await userEvent.click(screen.getByRole('button', { name: 'Sign out' }));

    await loginHeading();
    expect(live().length).toBe(0);
    expect(calls.some((c) => c.key === 'POST /api/auth/logout')).toBe(true);
  });

  it('stops when an API call is answered 401', async () => {
    mockFetch({ 'GET /api/auth/state': () => json(inState()), 'GET /api/settings': () => new Response(null, { status: 401 }) });
    renderGate();
    await screen.findByText('The app');
    await waitFor(() => expect(live().length).toBe(1));

    await expect(getJson('/api/settings')).rejects.toThrow();

    await loginHeading();
    expect(live().length).toBe(0);
  });

  it('asks the server whether the session is still good when the stream closes', async () => {
    let signedIn = true;
    const { count } = mockFetch({ 'GET /api/auth/state': () => json(inState({ signedIn })) });
    renderGate();
    await screen.findByText('The app');
    await waitFor(() => expect(live().length).toBe(1));
    // The initial load alone: the first open needs no refetch.
    expect(count('GET /api/auth/state')).toBe(1);

    // The server ends the stream when the session ends, and refuses a new one.
    signedIn = false;
    FakeEventSource.answering = false;
    act(() => FakeEventSource.instances[0].fail());

    await loginHeading();
    expect(count('GET /api/auth/state')).toBeGreaterThanOrEqual(2);
  });

  it('shows the login screen when the session ended during a long outage, on a later retry', async () => {
    let signedIn = true;
    const { count } = mockFetch({ 'GET /api/auth/state': () => json(inState({ signedIn })) });
    renderGate();
    await screen.findByText('The app');
    await waitFor(() => expect(live().length).toBe(1));

    // The server goes away: each try asks whether the session is still good, and retries go on.
    FakeEventSource.answering = false;
    act(() => FakeEventSource.instances[0].fail());
    await waitFor(() => expect(FakeEventSource.instances.length).toBe(2));
    await waitFor(() => expect(count('GET /api/auth/state')).toBe(3));
    expect(screen.getByText('The app')).toBeTruthy();

    // The session ends meanwhile; the next try, two seconds on, finds out.
    signedIn = false;
    await screen.findByRole('heading', { name: 'Sign in to bench-pc' }, { timeout: 5000 });
    expect(FakeEventSource.instances.length).toBe(3);
  });

  it('opens a new stream and fetches everything again once the server is back', async () => {
    const { count } = mockFetch({ 'GET /api/auth/state': () => json(inState()) });
    renderGate();
    await screen.findByText('The app');
    await waitFor(() => expect(live().length).toBe(1));

    act(() => FakeEventSource.instances[0].fail());

    await waitFor(() => expect(live().length).toBe(1));
    expect(live()[0]).toBe(FakeEventSource.instances[1]);
    // The initial load, the recheck, and the refetch after the new stream opened.
    await waitFor(() => expect(count('GET /api/auth/state')).toBe(3));
  });

  it('does not bring the app back when a recheck or a late auth answer lands during sign-out', async () => {
    const logout = deferred<Response>();
    const lateState = deferred<Response>();
    let stateCalls = 0;
    mockFetch({
      // The first answer is the initial load; the second is a
      // recheck still in flight when sign-out starts.
      'GET /api/auth/state': () => (++stateCalls <= 1 ? json(inState()) : lateState.promise),
      'POST /api/auth/logout': () => logout.promise,
    });
    renderGate();
    await screen.findByText('The app');
    await waitFor(() => expect(stateCalls).toBe(1));
    await userEvent.click(screen.getByRole('button', { name: 'Recheck' }));
    await waitFor(() => expect(stateCalls).toBe(2));

    FakeEventSource.answering = false;
    await userEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    // The server ends the stream as the session ends, and the retry fails: those rechecks are ignored.
    act(() => FakeEventSource.instances[0].fail());
    await waitFor(() => expect(FakeEventSource.instances.length).toBe(2));
    expect(stateCalls).toBe(2);
    await act(async () => {
      logout.resolve(noContent());
      lateState.resolve(json(inState()));
      await logout.promise;
    });

    await loginHeading();
    expect(screen.queryByText('The app')).toBeNull();
  });
});
