import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { getJson } from '@/api/http';
import { json, mockFetch } from './harness';
import { busy, noContent, problem, renderGate, state } from './auth-fixtures';

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
