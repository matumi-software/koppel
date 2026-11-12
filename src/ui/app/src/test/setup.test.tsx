import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '@/app';
import { SetupPage } from '@/setup/setup-page';
import { accepted, json, mockFetch, renderApp } from './harness';
import { baseRoutes, settings } from './fixtures';

afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
});

describe('setup', () => {
  it('sends the role and setupComplete in one PATCH', async () => {
    const { calls } = mockFetch({ ...baseRoutes, 'GET /api/settings': () => json({ ...settings, setupComplete: false }), 'PATCH /api/settings': () => accepted() });
    renderApp(<SetupPage locked={false} />);
    await screen.findByRole('button', { name: 'Continue' });

    await userEvent.click(screen.getByRole('radio', { name: /Client/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Continue' }));

    const patches = calls.filter((c) => c.key === 'PATCH /api/settings');
    expect(patches).toHaveLength(1);
    expect(patches[0].body).toEqual({ role: 'client', setupComplete: true });
  });

  it('stays pending until the settings show both the role and setupComplete', async () => {
    mockFetch({ ...baseRoutes, 'GET /api/settings': () => json({ ...settings, setupComplete: false }), 'PATCH /api/settings': () => accepted() });
    const { live } = renderApp(<SetupPage locked={false} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Continue' }));
    await screen.findByRole('button', { name: 'Saving' });

    await act(async () => live.emit('SettingsChanged', { ...settings, setupComplete: true, role: 'client' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Saving' })).toBeTruthy());

    await act(async () => live.emit('SettingsChanged', { ...settings, setupComplete: true, role: 'server' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Saving' })).toBeNull());
  });
});

describe('first run in the app', () => {
  const unconfigured = () => mockFetch({ ...baseRoutes, 'GET /api/settings': () => json({ ...settings, setupComplete: false }), 'PATCH /api/settings': () => accepted() });

  it('stays on the setup page, address untouched, until the server confirms, then lands on the page the role needs', async () => {
    unconfigured();
    const { live } = renderApp(<App />);
    await userEvent.click(await screen.findByRole('radio', { name: /Client/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Continue' }));
    await screen.findByRole('button', { name: 'Saving' });

    expect(window.location.hash).toBe('');
    expect(screen.getByRole('heading', { name: /Set up/ })).toBeTruthy();

    await act(async () => live.emit('SettingsChanged', { ...settings, setupComplete: true, role: 'client' }));

    await screen.findByRole('heading', { name: 'Configuration' });
  });

  it('lands a server on its clients', async () => {
    unconfigured();
    const { live } = renderApp(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Continue' }));
    await screen.findByRole('button', { name: 'Saving' });

    await act(async () => live.emit('SettingsChanged', { ...settings, setupComplete: true, role: 'server' }));

    await screen.findByRole('heading', { name: 'Clients' });
  });

  it('does not leave the setup page when the server refuses', async () => {
    mockFetch({ ...baseRoutes, 'GET /api/settings': () => json({ ...settings, setupComplete: false }), 'PATCH /api/settings': () => json({ errors: { role: ['No.'] } }, 400) });
    renderApp(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Continue' }));

    await screen.findByText('Not saved: No.');
    expect(window.location.hash).toBe('');
  });
});
