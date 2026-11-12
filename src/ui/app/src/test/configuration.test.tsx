import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { KoppelSettings } from '@/api/settings';
import { confirmTimeoutMs } from '@/pending/domains';
import { accepted, announced, json, mockFetch, renderApp } from './harness';
import { baseRoutes, configuration, connection, fingerprint, settings } from './fixtures';
import { systemInfo } from './wire';

afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
});

describe('the role', () => {
  it('shows a role it does not know as Unknown and does not treat it as both', async () => {
    mockFetch({ ...baseRoutes, 'GET /api/v1/settings': () => json({ ...settings, role: 'gateway' }) });
    renderApp(configuration());

    await screen.findByText(/this version of Koppel does not know it/);
    expect(screen.getByText(/role is Unknown/)).toBeTruthy();
    expect(screen.queryByText('Sharing')).toBeNull();
    expect(screen.queryByText('Server connection')).toBeNull();
  });

  it('shows Saving and Saved, and says how the change ended through the live region', async () => {
    mockFetch({ ...baseRoutes, 'PATCH /api/v1/settings': () => accepted() });
    const { live } = renderApp(configuration());
    await screen.findByText('Role');

    fireEvent.click(screen.getByRole('radio', { name: /Both/ }));
    await screen.findByText('Saving…');

    await act(async () => live.emit('SettingsChanged', { ...settings, role: 'both' }));
    await screen.findByText('Saved');
    expect(announced()).toBe('Role: both. Saved.');
  });
});

describe('notifications', () => {
  const withTray = (role: KoppelSettings['role']) => ({
    ...baseRoutes,
    'GET /api/v1/settings': () => json({ ...settings, role }),
    'GET /api/v1/system/info': () =>
      json({ ...systemInfo, role, os: 'Windows', tray: true }),
    'PATCH /api/v1/settings': () => accepted(),
  });

  it('are not offered without a tray icon', async () => {
    mockFetch(baseRoutes);
    renderApp(configuration());
    await screen.findByText('Role');

    expect(screen.queryByText('Notifications')).toBeNull();
  });

  it('offer pairing on a server, and the link and failed attaches on a client', async () => {
    mockFetch(withTray('server'));
    const { unmount } = renderApp(configuration());

    await screen.findByRole('switch', { name: /When a client wants to pair/ });
    expect(screen.queryByRole('switch', { name: /When the link to the server drops/ })).toBeNull();
    unmount();

    mockFetch(withTray('client'));
    renderApp(configuration());
    await screen.findByRole('switch', { name: /When the link to the server drops/ });
    expect(screen.getByRole('switch', { name: /When a device does not attach/ })).toBeTruthy();
    expect(screen.queryByRole('switch', { name: /When a client wants to pair/ })).toBeNull();
  });

  it('turn one off with a PATCH of that setting, confirmed by the push', async () => {
    const { calls } = mockFetch(withTray('client'));
    const { live } = renderApp(configuration());

    fireEvent.click(await screen.findByRole('switch', { name: /When the link to the server drops/ }));
    await waitFor(() => expect(calls.filter((c) => c.key === 'PATCH /api/v1/settings').map((c) => c.body)).toEqual([{ notifyLink: false }]));

    await act(async () => live.emit('SettingsChanged', { ...settings, role: 'client', notifyLink: false }));
    await waitFor(() => expect((screen.getByRole('switch', { name: /When the link to the server drops/ }) as HTMLInputElement).checked).toBe(false));
  });
});

describe('the server connection', () => {
  const secret = () => screen.getByLabelText('Client secret') as HTMLInputElement;
  const clientId = () => screen.getByLabelText('Client ID') as HTMLInputElement;
  const saveConnection = () => screen.getByRole('button', { name: 'Save connection' }) as HTMLButtonElement;
  const testButton = () => screen.getByRole('button', { name: 'Test connection' }) as HTMLButtonElement;

  const clientRole = { ...baseRoutes, 'GET /api/v1/settings': () => json({ ...settings, role: 'client' }) };

  it('is confirmed only by a push that carries what was sent, then clears the secret and enables Test', async () => {
    const { calls } = mockFetch({ ...clientRole, 'PUT /api/v1/client/connection': () => accepted() });
    const { live } = renderApp(configuration());
    await screen.findByLabelText('Client secret');

    await userEvent.clear(clientId());
    await userEvent.type(clientId(), 'bench-2');
    await userEvent.type(secret(), 'hunter2');
    expect(testButton().disabled).toBe(true);
    await userEvent.click(saveConnection());
    expect(calls.find((c) => c.key === 'PUT /api/v1/client/connection')?.body).toMatchObject({ clientId: 'bench-2', clientSecret: 'hunter2' });
    await waitFor(() => expect(clientId().disabled).toBe(true));

    // A push for some other connection does not confirm it: the save stays pending.
    await act(async () => live.emit('ConnectionChanged', { ...connection, clientId: 'someone-else' }));
    expect(clientId().disabled).toBe(true);
    expect(screen.queryByText(/Not saved/)).toBeNull();

    await act(async () => live.emit('ConnectionChanged', { ...connection, clientId: 'bench-2' }));

    await waitFor(() => expect(clientId().disabled).toBe(false));
    expect(secret().value).toBe('');
    expect(testButton().disabled).toBe(false);
  });

  it('clears the secret when only the secret was changed and the push repeats the stored values', async () => {
    mockFetch({ ...clientRole, 'PUT /api/v1/client/connection': () => accepted() });
    const { live } = renderApp(configuration());
    await screen.findByLabelText('Client secret');

    await userEvent.type(secret(), 'hunter2');
    await userEvent.click(saveConnection());
    await act(async () => live.emit('ConnectionChanged', connection));

    await waitFor(() => expect(secret().value).toBe(''));
    expect(testButton().disabled).toBe(false);
  });

  it('recognises the push when the server spells the address and fingerprint its own way', async () => {
    mockFetch({ ...clientRole, 'PUT /api/v1/client/connection': () => accepted() });
    const { live } = renderApp(configuration());
    await screen.findByLabelText('Client secret');

    await userEvent.clear(screen.getByLabelText('Server address'));
    await userEvent.type(screen.getByLabelText('Server address'), 'https://LAB.example:5241/some/path');
    await userEvent.clear(screen.getByLabelText('Server fingerprint'));
    await userEvent.type(screen.getByLabelText('Server fingerprint'), fingerprint.replaceAll(':', ' ').toLowerCase());
    await userEvent.click(saveConnection());
    await waitFor(() => expect(clientId().disabled).toBe(true));

    await act(async () => live.emit('ConnectionChanged', { ...connection, serverUrl: 'https://lab.example:5241' }));

    await waitFor(() => expect(clientId().disabled).toBe(false));
  });

  it('does not rebuild the form under someone typing when a save that had failed is confirmed late', async () => {
    mockFetch({ ...clientRole, 'PUT /api/v1/client/connection': () => accepted() });
    const { live } = renderApp(configuration());
    await screen.findByLabelText('Client secret');
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      fireEvent.change(clientId(), { target: { value: 'bench-2' } });
      fireEvent.click(saveConnection());
      await act(async () => {
        await vi.advanceTimersByTimeAsync(confirmTimeoutMs + 1);
      });
      expect(screen.getByText(/Not saved: no confirmation from the server\./)).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
    // Back to editing, then the server's confirmation of the old save arrives.
    fireEvent.change(secret(), { target: { value: 'hunter2' } });

    await act(async () => live.emit('ConnectionChanged', { ...connection, clientId: 'bench-2' }));

    expect(secret().value).toBe('hunter2');
    expect(screen.queryByText(/Not saved/)).toBeNull();
  });

  it('shows why a test call failed instead of dropping the rejection', async () => {
    mockFetch({ ...clientRole, 'POST /api/v1/client/connection/test': () => Promise.reject(new TypeError('Failed to fetch')) as unknown as Response });
    renderApp(configuration());
    await screen.findByLabelText('Client secret');

    await userEvent.click(testButton());

    await screen.findByText('Failed to fetch');
    expect(announced()).toBe('Connection test: Failed to fetch');
    expect(testButton().disabled).toBe(false);
  });
});
