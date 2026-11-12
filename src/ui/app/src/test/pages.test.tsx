import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ConnectionView } from '@/api/client';
import type { KoppelSettings } from '@/api/settings';
import { App } from '@/app';
import { AuthContext } from '@/auth/auth-context';
import { LiveContext } from '@/live/live-context';
import { confirmTimeoutMs } from '@/pending/domains';
import { ConfigurationPage } from '@/settings/configuration-page';
import { SetupPage } from '@/setup/setup-page';
import { FrameContext, type FrameApi } from '@/shell/frame-context';
import { accepted, deferred, json, mockFetch, renderApp } from './harness';

const settings: KoppelSettings = { role: 'server', shareOnStartup: false, setupComplete: true, notifyPairing: true, notifyLink: true, notifyAttachFailures: true };
const fingerprint = 'AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99';
const connection: ConnectionView = { serverUrl: 'https://lab:5241', serverFingerprint: fingerprint, clientId: 'bench-1', hasSecret: true, nextServerFingerprint: null };

const baseRoutes = {
  'GET /api/settings': () => json(settings),
  'GET /api/activity?max=200': () => json([]),
  'GET /api/server/identity': () => json({ name: 'bench-pc', fingerprint, httpsPort: 5241 }),
  'GET /api/system/checks': () => json([]),
  'GET /api/system/info': () =>
    json({ name: 'bench-pc', version: '1.0.0', startedAt: '2026-01-01T00:00:00Z', role: 'server', uiPort: 5240, httpsPort: 5241, fingerprint, os: 'test', tray: false }),
  'GET /api/client/discovery': () => json([]),
  'GET /api/client/pairing': () => json({ state: 'idle', serverName: null, serverUrl: null, code: null, message: null, expiresAt: null }),
  'GET /api/client/connection': () => json(connection),
  'GET /api/client/devices': () => json({ link: 'connected', linkMessage: null, devices: [], unavailable: null, gateway: false }),
  'GET /api/server/devices': () => json({ devices: [], unavailable: null }),
  'GET /api/server/clients': () => json([]),
  'GET /api/server/pairing': () => json({ budgetExhausted: false, requests: [] }),
};

const frame = (overrides: Partial<FrameApi> = {}): FrameApi => ({
  page: 'configuration',
  go: () => {},
  nav: null,
  navFoot: null,
  navFootState: 'live',
  status: null,
  isMenuOpen: false,
  setMenuOpen: () => {},
  locked: false,
  asideSlot: null,
  setTitle: () => {},
  ...overrides,
});

const configuration = (locked = false) => (
  <FrameContext.Provider value={frame({ locked })}>
    <ConfigurationPage />
  </FrameContext.Provider>
);

afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
});

describe('setup', () => {
  it('sends the role and setupComplete in one PATCH', async () => {
    const { calls } = mockFetch({ ...baseRoutes, 'GET /api/settings': () => json({ ...settings, setupComplete: false }), 'PATCH /api/settings': () => accepted() });
    renderApp(<SetupPage locked={false} onDone={() => {}} />);
    await screen.findByRole('button', { name: 'Continue' });

    await userEvent.click(screen.getByRole('radio', { name: /Client/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Continue' }));

    const patches = calls.filter((c) => c.key === 'PATCH /api/settings');
    expect(patches).toHaveLength(1);
    expect(patches[0].body).toEqual({ role: 'client', setupComplete: true });
  });

  it('stays pending until the settings show both the role and setupComplete', async () => {
    mockFetch({ ...baseRoutes, 'GET /api/settings': () => json({ ...settings, setupComplete: false }), 'PATCH /api/settings': () => accepted() });
    const { live } = renderApp(<SetupPage locked={false} onDone={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Continue' }));
    await screen.findByRole('button', { name: 'Saving' });

    await act(async () => live.emit('SettingsChanged', { ...settings, setupComplete: true, role: 'client' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Saving' })).toBeTruthy());

    await act(async () => live.emit('SettingsChanged', { ...settings, setupComplete: true, role: 'server' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Saving' })).toBeNull());
  });
});

describe('the open page', () => {
  beforeEach(() => {
    window.location.hash = '#attach';
  });

  it('is not redirected before the settings have loaded', async () => {
    const pending = deferred<Response>();
    mockFetch({ ...baseRoutes, 'GET /api/settings': () => pending.promise });
    renderApp(<App />);
    await act(async () => {});

    expect(window.location.hash).toBe('#attach');

    await act(async () => pending.resolve(json({ ...settings, role: 'client' })));
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    expect(window.location.hash).toBe('#attach');
  });

  it('is redirected to the overview once the loaded role has no such page', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);

    await waitFor(() => expect(window.location.hash).toBe('#overview'));
  });
});

describe('sign out', () => {
  it('says so when the server did not sign the browser out', async () => {
    window.location.hash = '#overview';
    mockFetch(baseRoutes);
    renderApp(
      <AuthContext.Provider value={{ recheck: () => {}, signOut: () => Promise.reject(new Error('the server answered 500.')) }}>
        <App />
      </AuthContext.Provider>,
    );

    await userEvent.click(await screen.findByRole('button', { name: 'Sign out' }));

    expect(await screen.findByText('Not signed out: the server answered 500.')).toBeTruthy();
  });
});

describe('the status card', () => {
  beforeEach(() => {
    window.location.hash = '#overview';
  });

  const card = () => document.querySelector('.kp-shell__side .kp-status-card') as HTMLElement;

  /** The app with the live link in a given state, over the harness's live link. */
  const appWithLink = (state: 'live' | 'reconnecting' | 'down', retry = () => {}) => (
    <LiveContext.Provider value={{ state, retry }}>
      <App />
    </LiveContext.Provider>
  );

  it('shows the machine name and the version from the system info, fetched once', async () => {
    const { count } = mockFetch(baseRoutes);
    renderApp(<App />);

    await waitFor(() => expect(within(card()).getByText('v1.0.0')).toBeTruthy());
    expect(within(card()).getByText('bench-pc')).toBeTruthy();
    expect(within(card()).getByRole('status').textContent).toBe('Live');
    expect(count('GET /api/system/info')).toBe(1);
  });

  it('says changes are paused while the link reconnects', async () => {
    mockFetch(baseRoutes);
    renderApp(appWithLink('reconnecting'));

    await waitFor(() => expect(card().dataset.state).toBe('reconnecting'));
    expect(within(card()).getByText('Changes are paused until the link is back.')).toBeTruthy();
    expect(within(card()).queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it('offers Retry while the link is down', async () => {
    mockFetch(baseRoutes);
    const retry = vi.fn();
    renderApp(appWithLink('down', retry));

    await waitFor(() => expect(card().dataset.state).toBe('down'));
    expect(within(card()).getByRole('alert').textContent).toBe('Disconnected');

    await userEvent.click(within(card()).getByRole('button', { name: 'Retry' }));

    expect(retry).toHaveBeenCalledOnce();
  });

  it.each([
    ['connected', 'Linked to lab.'],
    ['connecting', 'Connecting to lab…'],
    ['error', 'Cannot reach lab.'],
    ['unconfigured', 'No server connection yet.'],
  ] as const)('tells a client whose server link is %s what it is linked to', async (link, text) => {
    mockFetch({
      ...baseRoutes,
      'GET /api/settings': () => json({ ...settings, role: 'client' }),
      'GET /api/client/devices': () => json({ link, linkMessage: null, devices: [], unavailable: null, gateway: false }),
    });
    renderApp(<App />);

    await waitFor(() => expect(within(card()).getByText(text)).toBeTruthy());
  });

  it('does not say what a server is linked to', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);

    await waitFor(() => expect(within(card()).getByText('v1.0.0')).toBeTruthy());
    expect(within(card()).queryByText(/Linked to|Cannot reach|Connecting to|No server connection/)).toBeNull();
  });
});

describe('the role', () => {
  it('shows a role it does not know as Unknown and does not treat it as both', async () => {
    mockFetch({ ...baseRoutes, 'GET /api/settings': () => json({ ...settings, role: 'gateway' }) });
    renderApp(configuration());

    await screen.findByText(/this version of Koppel does not know it/);
    expect(screen.getByText(/role is Unknown/)).toBeTruthy();
    expect(screen.queryByText('Sharing')).toBeNull();
    expect(screen.queryByText('Server connection')).toBeNull();
  });

  it('announces Saving and Saved through a status region', async () => {
    mockFetch({ ...baseRoutes, 'PATCH /api/settings': () => accepted() });
    const { live } = renderApp(configuration());
    await screen.findByText('Role');

    fireEvent.click(screen.getByRole('radio', { name: /Both/ }));
    await waitFor(() => expect(screen.getByText('Saving…').getAttribute('role')).toBe('status'));

    await act(async () => live.emit('SettingsChanged', { ...settings, role: 'both' }));
    await waitFor(() => expect(screen.getByText('Saved').getAttribute('role')).toBe('status'));
  });
});

describe('notifications', () => {
  const withTray = (role: KoppelSettings['role']) => ({
    ...baseRoutes,
    'GET /api/settings': () => json({ ...settings, role }),
    'GET /api/system/info': () =>
      json({ name: 'bench-pc', version: '1.0.0', startedAt: '2026-01-01T00:00:00Z', role, uiPort: 5240, httpsPort: 5241, fingerprint, os: 'Windows', tray: true }),
    'PATCH /api/settings': () => accepted(),
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
    await waitFor(() => expect(calls.filter((c) => c.key === 'PATCH /api/settings').map((c) => c.body)).toEqual([{ notifyLink: false }]));

    await act(async () => live.emit('SettingsChanged', { ...settings, role: 'client', notifyLink: false }));
    await waitFor(() => expect((screen.getByRole('switch', { name: /When the link to the server drops/ }) as HTMLInputElement).checked).toBe(false));
  });
});

describe('the server connection', () => {
  const secret = () => screen.getByLabelText('Client secret') as HTMLInputElement;
  const clientId = () => screen.getByLabelText('Client ID') as HTMLInputElement;
  const saveConnection = () => screen.getByRole('button', { name: 'Save connection' }) as HTMLButtonElement;
  const testButton = () => screen.getByRole('button', { name: 'Test connection' }) as HTMLButtonElement;

  const clientRole = { ...baseRoutes, 'GET /api/settings': () => json({ ...settings, role: 'client' }) };

  it('is confirmed only by a push that carries what was sent, then clears the secret and enables Test', async () => {
    const { calls } = mockFetch({ ...clientRole, 'PUT /api/client/connection': () => accepted() });
    const { live } = renderApp(configuration());
    await screen.findByLabelText('Client secret');

    await userEvent.clear(clientId());
    await userEvent.type(clientId(), 'bench-2');
    await userEvent.type(secret(), 'hunter2');
    expect(testButton().disabled).toBe(true);
    await userEvent.click(saveConnection());
    expect(calls.find((c) => c.key === 'PUT /api/client/connection')?.body).toMatchObject({ clientId: 'bench-2', clientSecret: 'hunter2' });
    await waitFor(() => expect(clientId().disabled).toBe(true));

    // A push for some other connection does not confirm it.
    await act(async () => live.emit('ConnectionChanged', { ...connection, clientId: 'someone-else' }));
    await waitFor(() => expect(clientId().value).toBe('someone-else'));
    expect(screen.queryByText(/Not saved/)).toBeNull();

    await act(async () => live.emit('ConnectionChanged', { ...connection, clientId: 'bench-2' }));

    await waitFor(() => expect(clientId().disabled).toBe(false));
    expect(secret().value).toBe('');
    expect(testButton().disabled).toBe(false);
  });

  it('clears the secret when only the secret was changed and the push repeats the stored values', async () => {
    mockFetch({ ...clientRole, 'PUT /api/client/connection': () => accepted() });
    const { live } = renderApp(configuration());
    await screen.findByLabelText('Client secret');

    await userEvent.type(secret(), 'hunter2');
    await userEvent.click(saveConnection());
    await act(async () => live.emit('ConnectionChanged', connection));

    await waitFor(() => expect(secret().value).toBe(''));
    expect(testButton().disabled).toBe(false);
  });

  it('recognises the push when the server spells the address and fingerprint its own way', async () => {
    mockFetch({ ...clientRole, 'PUT /api/client/connection': () => accepted() });
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
    mockFetch({ ...clientRole, 'PUT /api/client/connection': () => accepted() });
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
    mockFetch({ ...clientRole, 'POST /api/client/connection/test': () => Promise.reject(new TypeError('Failed to fetch')) as unknown as Response });
    renderApp(configuration());
    await screen.findByLabelText('Client secret');

    await userEvent.click(testButton());

    expect((await screen.findByText('Failed to fetch')).getAttribute('role')).toBe('status');
    expect(testButton().disabled).toBe(false);
  });
});
