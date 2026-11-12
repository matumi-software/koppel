import { QueryClientProvider } from '@tanstack/react-query';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AuthState } from '@/api/auth';
import { getJson } from '@/api/http';
import { AuthGate } from '@/auth/auth-gate';
import { useAuth } from '@/auth/auth-context';
import { LiveProvider } from '@/live/live-provider';
import { eventsUrl } from '@/live/push-table';
import { ChangePassword } from '@/settings/change-password';
import { FakeEventSource } from './fake-event-source';
import { deferred, json, mockFetch, newQueryClient, renderApp } from './harness';

const noContent = () => new Response(null, { status: 204 });
const problem = (error: string, status: number) => json({ error }, status);

const busy = 'The server is busy checking another password. Try again in a moment.';

const state = (overrides: Partial<AuthState> = {}): AuthState => ({ signedIn: false, passwordSet: true, machineName: 'bench-pc', setupOpen: false, setupUntil: null, ...overrides });

/** A machine without a password, still taking its first one. */
const openSetup = (overrides: Partial<AuthState> = {}) => state({ passwordSet: false, setupOpen: true, setupUntil: '2026-11-03T12:40:00Z', ...overrides });

function SignOutButton() {
  const { signOut } = useAuth();
  return <button onClick={() => void signOut()}>Sign out</button>;
}

function RecheckButton() {
  const { recheck } = useAuth();
  return <button onClick={recheck}>Recheck</button>;
}

function renderGate() {
  const queryClient = newQueryClient();
  render(
    <QueryClientProvider client={queryClient}>
      <AuthGate>
        <LiveProvider>
          <p>The app</p>
          <SignOutButton />
          <RecheckButton />
        </LiveProvider>
      </AuthGate>
    </QueryClientProvider>,
  );
}

const live = () => FakeEventSource.live;
const loginHeading = () => screen.findByRole('heading', { name: 'Sign in to bench-pc' });

afterEach(() => vi.unstubAllGlobals());

describe('login', () => {
  it('shows the login screen, not the app, when the browser is not signed in', async () => {
    mockFetch({ 'GET /api/auth/state': () => json(state()) });
    renderGate();

    await screen.findByRole('heading', { name: 'Sign in to bench-pc' });

    expect(screen.queryByText('The app')).toBeNull();
  });

  it('sends the password with the UI header and shows the app once the server says signed in', async () => {
    let signedIn = false;
    const { calls, fetchMock } = mockFetch({
      'GET /api/auth/state': () => json(state({ signedIn })),
      'POST /api/auth/login': () => {
        signedIn = true;
        return noContent();
      },
    });
    renderGate();
    await userEvent.type(await screen.findByLabelText('Password'), 'correct horse battery');

    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));

    await screen.findByText('The app');
    expect(calls.find((c) => c.key === 'POST /api/auth/login')?.body).toEqual({ password: 'correct horse battery' });
    const login = fetchMock.mock.calls.find(([url]) => url === '/api/auth/login')!;
    expect(new Headers(login[1]?.headers).get('X-Koppel-UI')).toBe('1');
  });

  it('shows the server\'s answer to a wrong password and stays on the login screen', async () => {
    mockFetch({ 'GET /api/auth/state': () => json(state()), 'POST /api/auth/login': () => problem('That password is not right.', 401) });
    renderGate();
    await userEvent.type(await screen.findByLabelText('Password'), 'not the password');

    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));

    await screen.findByText('That password is not right.');
    expect(screen.queryByText('The app')).toBeNull();
  });

  it('signs in with the right password after a wrong one, without a reload', async () => {
    let signedIn = false;
    const { calls } = mockFetch({
      'GET /api/auth/state': () => json(state({ signedIn })),
      'POST /api/auth/login': (init) => {
        if ((JSON.parse(String(init?.body)) as { password: string }).password !== 'correct horse battery') return problem('That password is not right.', 401);
        signedIn = true;
        return noContent();
      },
    });
    renderGate();
    const field = await screen.findByLabelText('Password');
    await userEvent.type(field, 'not the password');
    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));
    await screen.findByText('That password is not right.');

    await userEvent.clear(field);
    await userEvent.type(field, 'correct horse battery');
    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));

    await screen.findByText('The app');
    expect(calls.filter((c) => c.key === 'POST /api/auth/login')).toHaveLength(2);
  });

  it('shows how long to wait when the server says too many tries', async () => {
    mockFetch({
      'GET /api/auth/state': () => json(state()),
      'POST /api/auth/login': () => problem('Too many tries. Wait 1 minutes and try again.', 429),
    });
    renderGate();
    await userEvent.type(await screen.findByLabelText('Password'), 'whatever it is');

    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));

    await screen.findByText(/Too many tries/);
  });

  it('shows the busy answer on login and keeps what was typed', async () => {
    mockFetch({
      'GET /api/auth/state': () => json(state()),
      'POST /api/auth/login': () => problem(busy, 429),
    });
    renderGate();
    await userEvent.type(await screen.findByLabelText('Password'), 'whatever it is');

    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));

    await screen.findByText(busy);
    expect((screen.getByLabelText('Password') as HTMLInputElement).value).toBe('whatever it is');
  });

  it('goes back to the login screen when an API call is answered 401', async () => {
    mockFetch({ 'GET /api/auth/state': () => json(state({ signedIn: true })), 'GET /api/settings': () => new Response(null, { status: 401 }) });
    renderGate();
    await screen.findByText('The app');

    await expect(getJson('/api/settings')).rejects.toThrow();

    await screen.findByRole('heading', { name: 'Sign in to bench-pc' });
    expect(screen.queryByText('The app')).toBeNull();
  });
});

describe('first-run password', () => {
  it('asks only for a password and says until when setup is open', async () => {
    mockFetch({ 'GET /api/auth/state': () => json(openSetup()) });
    renderGate();

    await screen.findByRole('heading', { name: 'Set up bench-pc' });

    expect(screen.getByText(new RegExp(`Setup is open until ${new Date('2026-11-03T12:40:00Z').toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`))).toBeTruthy();
    expect(screen.queryByLabelText('Setup code')).toBeNull();
    expect(screen.queryByText(/setup-code\.txt|docker logs/)).toBeNull();
    expect(screen.queryByText('The app')).toBeNull();
  });

  it('keeps the button off until the password is long enough and repeated, then sends only the password', async () => {
    let passwordSet = false;
    const { calls } = mockFetch({
      'GET /api/auth/state': () => json(passwordSet ? state({ signedIn: true }) : openSetup()),
      'POST /api/auth/setup': () => {
        passwordSet = true;
        return noContent();
      },
    });
    renderGate();
    const button = await screen.findByRole('button', { name: 'Set password and continue' });
    await userEvent.type(screen.getByLabelText('New password'), 'short');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'short');
    expect(button.hasAttribute('disabled')).toBe(true);

    await userEvent.clear(screen.getByLabelText('New password'));
    await userEvent.clear(screen.getByLabelText('Repeat the password'));
    await userEvent.type(screen.getByLabelText('New password'), 'correct horse battery');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'correct horse battery');
    await userEvent.click(button);

    await screen.findByText('The app');
    expect(calls.find((c) => c.key === 'POST /api/auth/setup')?.body).toEqual({ password: 'correct horse battery' });
  });

  it('shows the server\'s refusal and stays while setup is still open', async () => {
    mockFetch({ 'GET /api/auth/state': () => json(openSetup()), 'POST /api/auth/setup': () => problem('Do not use "koppel" or "password" as the password.', 400) });
    renderGate();
    await userEvent.type(await screen.findByLabelText('New password'), 'koppel1234');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'koppel1234');

    await userEvent.click(screen.getByRole('button', { name: 'Set password and continue' }));

    await screen.findByText('Do not use "koppel" or "password" as the password.');
    expect(screen.getByRole('heading', { name: 'Set up bench-pc' })).toBeTruthy();
  });

  it('explains the timeout and how to restart once setup has closed', async () => {
    const { count } = mockFetch({ 'GET /api/auth/state': () => json(state({ passwordSet: false, setupUntil: '2026-11-03T12:40:00Z' })) });
    renderGate();

    await screen.findByRole('heading', { name: 'Setup has timed out' });

    expect(screen.getByText(/Restart Koppel, then open this page again/)).toBeTruthy();
    expect(screen.getByText('sudo systemctl restart koppel')).toBeTruthy();
    expect(screen.getByText('Restart Koppel', { selector: 'strong' })).toBeTruthy();
    expect(screen.queryByLabelText('New password')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Check again' }));
    await waitFor(() => expect(count('GET /api/auth/state')).toBe(2));
  });

  it('shows the timeout screen when a password comes too late', async () => {
    let closed = false;
    mockFetch({
      'GET /api/auth/state': () => json(closed ? state({ passwordSet: false, setupUntil: '2026-11-03T12:40:00Z' }) : openSetup()),
      'POST /api/auth/setup': () => {
        closed = true;
        return problem('Setup has timed out. Restart Koppel, then set it.', 403);
      },
    });
    renderGate();
    await userEvent.type(await screen.findByLabelText('New password'), 'correct horse battery');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'correct horse battery');

    await userEvent.click(screen.getByRole('button', { name: 'Set password and continue' }));

    await screen.findByRole('heading', { name: 'Setup has timed out' });
  });
});

describe('change password', () => {
  const fill = async (current: string, next: string) => {
    await userEvent.type(screen.getByLabelText('Current password'), current);
    await userEvent.type(screen.getByLabelText('New password'), next);
    await userEvent.type(screen.getByLabelText('Repeat the new password'), next);
  };

  it('completes on the REST response, with the UI header, and clears the fields', async () => {
    const { fetchMock, calls } = mockFetch({ 'POST /api/auth/password': () => noContent() });
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText('Saved. Other browsers are signed out.');
    expect(calls.find((c) => c.key === 'POST /api/auth/password')?.body).toEqual({ current: 'correct horse battery', next: 'another long password' });
    expect(new Headers(fetchMock.mock.calls.find(([url]) => url === '/api/auth/password')![1]?.headers).get('X-Koppel-UI')).toBe('1');
    expect((screen.getByLabelText('Current password') as HTMLInputElement).value).toBe('');
  });

  it('shows the server\'s refusal and keeps what was typed', async () => {
    mockFetch({ 'POST /api/auth/password': () => problem('The current password is not right.', 400) });
    renderApp(<ChangePassword locked={false} />);
    await fill('not the password', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText('Not saved: The current password is not right.');
    expect((screen.getByLabelText('New password') as HTMLInputElement).value).toBe('another long password');
  });

  it('shows the busy answer and keeps what was typed', async () => {
    mockFetch({ 'POST /api/auth/password': () => problem(busy, 429) });
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText(`Not saved: ${busy}`);
    expect((screen.getByLabelText('New password') as HTMLInputElement).value).toBe('another long password');
  });

  it('is disabled while the link is down and until the new password is long enough', async () => {
    mockFetch({});
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'short');

    await waitFor(() => expect(screen.getByRole('button', { name: 'Change password' }).hasAttribute('disabled')).toBe(true));
  });
});

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
