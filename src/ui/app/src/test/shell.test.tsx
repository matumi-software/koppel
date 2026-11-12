import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '@/app';
import { AuthContext } from '@/auth/auth-context';
import { LiveContext } from '@/live/live-context';
import { deferred, json, mockFetch, renderApp } from './harness';
import { baseRoutes, settings } from './fixtures';

afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
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

  it('shows the overview once the loaded role has no such page, without touching the address', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);

    await screen.findByRole('heading', { name: 'Overview' });
    expect(window.location.hash).toBe('#attach');
    expect(document.querySelector('.kp-shell__top')?.textContent).toContain('Overview');
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
    expect(within(card()).getByText('Live')).toBeTruthy();
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
    expect(within(card()).getByText('Disconnected')).toBeTruthy();

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
