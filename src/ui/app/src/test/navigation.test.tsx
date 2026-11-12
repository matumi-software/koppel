import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '@/app';
import { AuthContext } from '@/auth/auth-context';
import { ShellBoundary } from '@/shell/shell-boundary';
import { baseRoutes } from './fixtures';
import { json, mockFetch, renderApp } from './harness';

beforeEach(async () => {
  window.location.hash = '#overview';
  // The address change is announced to the page a tick later: let it pass before the app opens.
  await new Promise((resolve) => setTimeout(resolve, 0));
});
afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
});

/** The live regions the app draws. react-aria keeps its own off-screen announcer for selection changes: that one is not the app's. */
const appLiveRegions = () => [...document.querySelectorAll('[role="status"], [role="alert"], [role="log"], [aria-live]')].filter((el) => !el.closest('[data-live-announcer]'));

const sidebar = () => document.querySelector('.kp-shell__side') as HTMLElement;

describe('navigation', () => {
  it('is made of real links, so a page can open in a new tab', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);
    await screen.findByRole('heading', { level: 1, name: 'Overview' });

    expect(within(sidebar()).getByRole('link', { name: /^Devices/ }).getAttribute('href')).toBe('#devices');
    expect(within(sidebar()).getByRole('link', { name: /Clients/ }).getAttribute('href')).toBe('#clients');
    expect(within(sidebar()).getByRole('link', { name: 'Configuration' }).getAttribute('href')).toBe('#configuration');
    expect(within(sidebar()).getByRole('link', { name: /Overview/ }).getAttribute('aria-current')).toBe('page');
  });

  it('opens the page a link points to, sets the tab title, and moves focus to the new heading', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);
    await screen.findByRole('heading', { level: 1, name: 'Overview' });
    expect(document.title).toBe('Overview - Koppel');

    await userEvent.click(within(sidebar()).getByRole('link', { name: 'Configuration' }));

    const heading = await screen.findByRole('heading', { level: 1, name: 'Configuration' });
    await waitFor(() => expect(document.activeElement).toBe(heading));
    expect(document.title).toBe('Configuration - Koppel');
    expect(within(sidebar()).getByRole('link', { name: 'Configuration' }).getAttribute('aria-current')).toBe('page');
  });

  it('leaves focus alone on the first page of a visit', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);

    const heading = await screen.findByRole('heading', { level: 1, name: 'Overview' });

    expect(document.activeElement).not.toBe(heading);
  });
});

describe('the live region', () => {
  it('is the only live region in the app', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);
    await screen.findByRole('heading', { level: 1, name: 'Overview' });

    expect(appLiveRegions()).toHaveLength(1);
  });

  it('is still the only one with a device selected and a failure showing', async () => {
    window.location.hash = '#devices';
    mockFetch({
      ...baseRoutes,
      'GET /api/v1/server/devices': () =>
        json({
          devices: [{ busId: '1-2', vidPid: '0403:6001', name: 'Serial adapter', manufacturer: null, speed: '480', state: 'local', attachedTo: null, attachedSince: null, fault: 'The kernel refused it.', serial: null, driver: null, interfaces: null }],
          unavailable: null,
        }),
    });
    renderApp(<App />);

    await userEvent.click(await screen.findByText('Serial adapter'));

    await screen.findByText('Not saved');
    expect(appLiveRegions()).toHaveLength(1);
  });
});

describe('a render error', () => {
  it('shows Reload and Sign out instead of a blank page', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    mockFetch(baseRoutes);
    function Broken(): never {
      throw new Error('boom');
    }

    renderApp(
      <AuthContext.Provider value={{ recheck: () => {}, signOut: () => Promise.resolve() }}>
        <ShellBoundary>
          <Broken />
        </ShellBoundary>
      </AuthContext.Provider>,
    );

    expect(await screen.findByRole('heading', { name: 'Something went wrong' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Reload' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Sign out' })).toBeTruthy();
    error.mockRestore();
  });
});
