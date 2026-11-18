import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '@/app';
import { AuthContext } from '@/auth/auth-context';
import { ShellBoundary } from '@/shell/shell-boundary';
import { baseRoutes } from './fixtures';
import { mockFetch, renderApp } from './harness';

beforeEach(async () => {
  window.location.hash = '#overview';
  // The address change is announced to the page a tick later: let it pass before the app opens.
  await new Promise((resolve) => setTimeout(resolve, 0));
});
afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
});

/** The one live region: Heartwood's, which toasts and announce() both speak through. */
const liveRegions = () => [...document.querySelectorAll('[role="region"][aria-label="Notifications"]')];

describe('navigation', () => {
  it('opens the page a nav item names, sets the tab title, and moves focus to the new heading', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);
    await screen.findByRole('heading', { level: 1, name: 'bench-pc' });
    expect(document.title).toBe('Overview · Koppel');

    await userEvent.click(await screen.findByRole('link', { name: 'Settings' }));

    const heading = await screen.findByRole('heading', { level: 1, name: 'Settings' });
    await waitFor(() => expect(document.activeElement).toBe(heading));
    expect(window.location.hash).toBe('#settings');
    expect(document.title).toBe('Settings · Koppel');
  });

  it('leaves focus alone on the first page of a visit', async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);

    const heading = await screen.findByRole('heading', { level: 1, name: 'bench-pc' });

    expect(document.activeElement).not.toBe(heading);
  });
});

describe('the live region', () => {
  it("is Heartwood's, drawn once", async () => {
    mockFetch(baseRoutes);
    renderApp(<App />);
    await screen.findByRole('heading', { level: 1, name: 'bench-pc' });

    expect(liveRegions()).toHaveLength(1);
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
