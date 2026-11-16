import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { OverviewPage } from '@/overview/overview-page';
import { accepted, json, mockFetch, renderApp } from './harness';
import { baseRoutes, inFrame } from './fixtures';
import { clientDevices, clients, pairingOverview, serverDevices, settings, systemChecks } from './wire';

afterEach(() => vi.unstubAllGlobals());

const routes = {
  ...baseRoutes,
  'GET /api/v1/settings': () => json({ ...settings, role: 'both' }),
  'GET /api/v1/server/devices': () => json(serverDevices),
  'GET /api/v1/server/clients': () => json(clients),
  'GET /api/v1/client/devices': () => json(clientDevices),
  'GET /api/v1/system/checks': () => json(systemChecks),
  'GET /api/v1/server/pairing': () => json(pairingOverview),
};

describe('the overview', () => {
  it('says what needs the person: a fault, a failing check and a waiting request', async () => {
    mockFetch(routes);
    renderApp(inFrame(<OverviewPage />));

    expect(await screen.findByText('Flash drive is not shared')).toBeTruthy();
    expect(screen.getByText('The device is busy.')).toBeTruthy();
    expect(screen.getByText('Attaching needs vhci-hcd.')).toBeTruthy();
    expect(screen.getByText('bench-laptop wants to pair')).toBeTruthy();
    expect(screen.getByText(/3 things need you\./)).toBeTruthy();
  });

  it('shares a faulted device again from Needs you, pending until confirmed', async () => {
    const { calls } = mockFetch({ ...routes, 'PUT /api/v1/server/devices/1-2/shared': () => accepted() });
    renderApp(inFrame(<OverviewPage />));

    await userEvent.click(await screen.findByRole('button', { name: 'Share Flash drive again' }));

    await waitFor(() => expect(calls.find((c) => c.key === 'PUT /api/v1/server/devices/1-2/shared')?.body).toEqual({ shared: true }));
  });

  it('opens a device in place and goes back to the overview', async () => {
    mockFetch(routes);
    renderApp(inFrame(<OverviewPage />));
    const tile = (await screen.findByRole('listbox', { name: /Devices on/ })) as HTMLElement;

    await userEvent.click(within(tile).getByRole('option', { name: /USB Receiver/ }));

    expect(await screen.findByRole('heading', { level: 1, name: 'USB Receiver' })).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Overview' }));
    expect(await screen.findByRole('listbox', { name: /Devices on/ })).toBeTruthy();
  });

  it('cannot send a share while the live link is down', async () => {
    mockFetch(routes);
    renderApp(inFrame(<OverviewPage />, { locked: true }));

    const again = await screen.findByRole('button', { name: 'Share Flash drive again' });

    expect((again as HTMLButtonElement).disabled).toBe(true);
  });
});
