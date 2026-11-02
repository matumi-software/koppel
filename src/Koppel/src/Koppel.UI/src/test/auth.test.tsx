import { QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AuthState } from '@/api/auth';
import { getJson } from '@/api/http';
import { AuthGate } from '@/auth/auth-gate';
import { ChangePassword } from '@/settings/change-password';
import { json, mockFetch, newQueryClient, renderApp } from './harness';

const noContent = () => new Response(null, { status: 204 });
const problem = (error: string, status: number) => json({ error }, status);

const state = (overrides: Partial<AuthState> = {}): AuthState => ({ signedIn: false, passwordSet: true, machineName: 'bench-pc', ...overrides });

function renderGate() {
  const queryClient = newQueryClient();
  render(
    <QueryClientProvider client={queryClient}>
      <AuthGate>
        <p>The app</p>
      </AuthGate>
    </QueryClientProvider>,
  );
  return queryClient;
}

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
  it('asks for the setup code and says where to find it', async () => {
    mockFetch({ 'GET /api/auth/state': () => json(state({ passwordSet: false })) });
    renderGate();

    await screen.findByRole('heading', { name: 'Set up bench-pc' });

    expect(screen.getByText(/docker logs/)).toBeTruthy();
    expect(screen.getByText(/setup-code\.txt/)).toBeTruthy();
    expect(screen.queryByText('The app')).toBeNull();
  });

  it('keeps the button off until the password is long enough and repeated, then sends the code and password', async () => {
    let passwordSet = false;
    const { calls } = mockFetch({
      'GET /api/auth/state': () => json(state({ passwordSet, signedIn: passwordSet })),
      'POST /api/auth/setup': () => {
        passwordSet = true;
        return noContent();
      },
    });
    renderGate();
    const button = await screen.findByRole('button', { name: 'Set password and continue' });
    await userEvent.type(screen.getByLabelText('Setup code'), 'ABCD-2345');
    await userEvent.type(screen.getByLabelText('New password'), 'short');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'short');
    expect(button.hasAttribute('disabled')).toBe(true);

    await userEvent.clear(screen.getByLabelText('New password'));
    await userEvent.clear(screen.getByLabelText('Repeat the password'));
    await userEvent.type(screen.getByLabelText('New password'), 'correct horse battery');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'correct horse battery');
    await userEvent.click(button);

    await screen.findByText('The app');
    expect(calls.find((c) => c.key === 'POST /api/auth/setup')?.body).toEqual({ setupCode: 'ABCD-2345', password: 'correct horse battery' });
  });

  it('shows a refused setup code', async () => {
    mockFetch({ 'GET /api/auth/state': () => json(state({ passwordSet: false })), 'POST /api/auth/setup': () => problem('That setup code is not right.', 403) });
    renderGate();
    await userEvent.type(await screen.findByLabelText('Setup code'), 'AAAA-AAAA');
    await userEvent.type(screen.getByLabelText('New password'), 'correct horse battery');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'correct horse battery');

    await userEvent.click(screen.getByRole('button', { name: 'Set password and continue' }));

    await screen.findByText('That setup code is not right.');
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

  it('is disabled while the link is down and until the new password is long enough', async () => {
    mockFetch({});
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'short');

    await waitFor(() => expect(screen.getByRole('button', { name: 'Change password' }).hasAttribute('disabled')).toBe(true));
  });
});
