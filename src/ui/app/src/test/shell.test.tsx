import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '@/app';
import { AuthContext } from '@/auth/auth-context';
import { LiveContext, type LiveState } from '@/live/live-context';
import { deferred, json, mockFetch, renderApp } from './harness';
import { baseRoutes, settings } from './fixtures';

afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
});

describe('the open page', () => {
  beforeEach(() => {
    window.location.hash = '#remote';
  });

  it('is not redirected before the settings have loaded', async () => {
    const pending = deferred<Response>();
    mockFetch({ ...baseRoutes, 'GET /api/v1/settings': () => pending.promise });
    renderApp(<App />);
    await act(async () => {});

    expect(window.location.hash).toBe('#remote');

    await act(async () => pending.resolve(json({ ...settings, role: 'client' })));
    await screen.findByRole('heading', { name: 'Add a server to attach its devices' });

    expect(window.location.hash).toBe('#remote');
  });

  it('shows the overview once the loaded role has no such page, without touching the address', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);

    await screen.findByRole('heading', { name: 'bench-pc' });
    expect(window.location.hash).toBe('#remote');
    expect(document.title).toBe('Overview - Koppel');
  });
});

describe('the navigation', () => {
  beforeEach(() => {
    window.location.hash = '#overview';
  });

  const navNames = () => [...document.querySelectorAll('.hw-nav-btn')].map((el) => el.textContent?.trim()).filter(Boolean);

  it.each([
    ['server', ['Overview', 'Devices', 'Activity', 'Clients', 'System check', 'Connection', 'Security', 'Settings']],
    ['client', ['Overview', 'Remote devices', 'Activity', 'System check', 'Connection', 'Security', 'Settings']],
    ['both', ['Overview', 'Devices', 'Remote devices', 'Activity', 'Clients', 'System check', 'Connection', 'Security', 'Settings']],
  ] as const)('has the places a %s has, in the prototype order', async (role, names) => {
    mockFetch({ ...baseRoutes, 'GET /api/v1/settings': () => json({ ...settings, role }) });
    renderApp(<App />);
    await screen.findByRole('heading', { name: 'bench-pc' });
    await waitFor(() => expect(navNames().filter((name) => name !== 'admin')).toEqual(names));
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

    await userEvent.click(await screen.findByRole('button', { name: 'admin' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Sign out' }));

    await waitFor(() => expect(document.querySelector('.kp-announcer')?.textContent).toBe('Signing out failed: the server answered 500.'));
  });
});

describe('the link state in the strip', () => {
  beforeEach(() => {
    window.location.hash = '#overview';
  });

  const appWithLink = (state: LiveState, retry = () => {}) => (
    <LiveContext.Provider value={{ state, retry }}>
      <App />
    </LiveContext.Provider>
  );

  it('says Live while the link is up', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);
    await screen.findByRole('heading', { name: 'bench-pc' });
    await waitFor(() => expect(screen.getAllByText('Live').length).toBeGreaterThan(0));
  });

  it('says changes are paused while the link reconnects', async () => {
    mockFetch(baseRoutes);
    renderApp(appWithLink('reconnecting'));

    await userEvent.click((await screen.findAllByRole('button', { name: /Reconnecting/ }))[0]);
    expect(await screen.findByText('Changes are paused')).toBeTruthy();
  });

  it('offers Try again while the link is down', async () => {
    mockFetch(baseRoutes);
    const retry = vi.fn();
    renderApp(appWithLink('down', retry));

    await userEvent.click((await screen.findAllByRole('button', { name: /Not connected/ }))[0]);
    await userEvent.click(await screen.findByRole('button', { name: 'Try again' }));

    expect(retry).toHaveBeenCalledOnce();
  });
});

describe('when Koppel does not answer', () => {
  it('says so and offers Try again', async () => {
    mockFetch({ ...baseRoutes, 'GET /api/v1/settings': () => Promise.resolve(new Response(null, { status: 500 })) });
    renderApp(<App />);

    expect(await screen.findByText('Could not reach Koppel')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Try again' })).toBeTruthy();
  });
});
