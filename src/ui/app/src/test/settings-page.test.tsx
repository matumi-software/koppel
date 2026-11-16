import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { KoppelSettings } from '@/api/settings';
import { SettingsPage } from '@/settings/settings-page';
import { baseRoutes, inFrame, settings } from './fixtures';
import { accepted, json, mockFetch, renderApp } from './harness';
import { systemInfo } from './wire';

afterEach(() => {
  vi.unstubAllGlobals();
  document.documentElement.removeAttribute('data-theme');
  window.localStorage.clear();
});

const routes = (over: Partial<KoppelSettings> = {}, tray = false) => ({
  ...baseRoutes,
  'GET /api/v1/settings': () => json({ ...settings, ...over }),
  'GET /api/v1/system/info': () => json({ ...systemInfo, tray, os: 'Windows 11' }),
  'PATCH /api/v1/settings': () => accepted(),
});

const shareSwitch = () => screen.findByRole('switch', { name: 'Share saved devices on startup' });

describe('the settings page', () => {
  it('stays pending until the push brings the value back, then confirms', async () => {
    const { calls } = mockFetch(routes());
    const { live } = renderApp(inFrame(<SettingsPage />));

    await userEvent.click(await shareSwitch());
    await waitFor(() => expect(calls.filter((c) => c.key === 'PATCH /api/v1/settings').map((c) => c.body)).toEqual([{ shareOnStartup: true }]));
    const control = screen.getByRole('switch', { name: 'Share saved devices on startup' }) as HTMLInputElement;
    expect(control.checked).toBe(false);
    expect(control.disabled).toBe(true);

    await act(async () => live.emit('SettingsChanged', { ...settings, shareOnStartup: true }));

    await waitFor(() => expect((screen.getByRole('switch', { name: 'Share saved devices on startup' }) as HTMLInputElement).checked).toBe(true));
    expect((screen.getByRole('switch', { name: 'Share saved devices on startup' }) as HTMLInputElement).disabled).toBe(false);
  });

  it('shows why a change was not saved when the server refuses it', async () => {
    mockFetch({ ...routes(), 'PATCH /api/v1/settings': () => new Response(null, { status: 500 }) });
    renderApp(inFrame(<SettingsPage />));

    await userEvent.click(await shareSwitch());

    await screen.findByText('Not saved: the server answered 500.');
  });

  it('changes the role with one PATCH and offers share on startup only to a server', async () => {
    const { calls } = mockFetch(routes());
    const { live } = renderApp(inFrame(<SettingsPage />));
    await shareSwitch();

    await userEvent.click(screen.getByRole('button', { name: /Role/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Client' }));
    await waitFor(() => expect(calls.filter((c) => c.key === 'PATCH /api/v1/settings').map((c) => c.body)).toEqual([{ role: 'client' }]));

    await act(async () => live.emit('SettingsChanged', { ...settings, role: 'client' }));
    await waitFor(() => expect(screen.queryByRole('switch', { name: 'Share saved devices on startup' })).toBeNull());
  });

  it('says so when the role is one this version does not know', async () => {
    mockFetch(routes({ role: 'gateway' as KoppelSettings['role'] }));
    renderApp(inFrame(<SettingsPage />));

    await screen.findByText(/does not know the role gateway/);
    expect(screen.queryByRole('switch', { name: 'Share saved devices on startup' })).toBeNull();
  });

  it('leaves out the notifications without a tray', async () => {
    mockFetch(routes());
    renderApp(inFrame(<SettingsPage />));
    await shareSwitch();
    await screen.findByText(/Windows 11/);

    expect(screen.queryByText('Notify me when')).toBeNull();
  });

  it('offers a client the link and attach notifications with a tray, and not the pairing one', async () => {
    mockFetch(routes({ role: 'client' }, true));
    renderApp(inFrame(<SettingsPage />));

    await screen.findByRole('switch', { name: 'The link to a server drops' });
    expect(screen.getByRole('switch', { name: 'A device does not attach' })).toBeTruthy();
    expect(screen.queryByRole('switch', { name: /A computer links to/ })).toBeNull();
  });

  it('disables a change while the link is down', async () => {
    mockFetch(routes());
    renderApp(inFrame(<SettingsPage />, { locked: true }));

    expect((await shareSwitch() as HTMLInputElement).disabled).toBe(true);
  });

  it('sets the theme in this browser without asking the server', async () => {
    const { calls } = mockFetch(routes());
    renderApp(inFrame(<SettingsPage />));
    await shareSwitch();

    await userEvent.click(within(screen.getByRole('radiogroup', { name: /theme/i })).getByRole('radio', { name: /Dark/ }));

    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(calls.some((c) => c.key.startsWith('PATCH'))).toBe(false);
  });

  it('lists what the UI uses with the version', async () => {
    mockFetch(routes());
    renderApp(inFrame(<SettingsPage />));
    await shareSwitch();

    expect(await screen.findByText(/1\.0\.0/)).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: /Third-party notices/ }));
    await screen.findByText('React Aria Components');
  });
});
