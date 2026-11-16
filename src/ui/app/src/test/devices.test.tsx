import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DevicesPage } from '@/devices/devices-page';
import { remoteEntries } from '@/devices/device-words';
import { RemoteDevicesPage } from '@/devices/remote-devices-page';
import { accepted, invalid, json, mockFetch, renderApp } from './harness';
import { baseRoutes, inFrame } from './fixtures';
import { clientDevices, clients, serverDevices } from './wire';

afterEach(() => vi.unstubAllGlobals());

const isOn = (el: HTMLElement) => (el as HTMLInputElement).checked;
const isOff = (el: HTMLElement) => (el as HTMLInputElement).disabled;

const flash = serverDevices.devices[1];
const receiver = serverDevices.devices[0];
const sharedFlash = { ...flash, state: 'shared', fault: null };

const serverRoutes = {
  ...baseRoutes,
  'GET /api/v1/server/devices': () => json(serverDevices),
  'GET /api/v1/server/clients': () => json(clients),
};

describe('the devices of this machine', () => {
  it('shows a share as pending until DevicesChanged brings it back', async () => {
    const { calls } = mockFetch({ ...serverRoutes, 'PUT /api/v1/server/devices/1-2/shared': () => accepted() });
    const { live } = renderApp(inFrame(<DevicesPage />));
    const toggle = await screen.findByRole('switch', { name: 'Share Flash drive' });
    expect(isOn(toggle)).toBe(false);

    await userEvent.click(toggle);

    await waitFor(() => expect(calls.find((c) => c.key === 'PUT /api/v1/server/devices/1-2/shared')?.body).toEqual({ shared: true }));
    // Asked for, not confirmed: the switch shows the target and the card still says what is true.
    expect(isOn(screen.getByRole('switch', { name: 'Share Flash drive' }))).toBe(true);
    expect(screen.getByText(/The device is busy\./)).toBeTruthy();

    await act(async () => live.emit('DevicesChanged', { ...serverDevices, devices: [receiver, sharedFlash] }));

    await waitFor(() => expect(screen.queryByText(/The device is busy\./)).toBeNull());
    expect(isOn(screen.getByRole('switch', { name: 'Share Flash drive' }))).toBe(true);
  });

  it('puts the switch back when the server refuses the share', async () => {
    mockFetch({ ...serverRoutes, 'PUT /api/v1/server/devices/1-2/shared': () => invalid('shared', 'The device is busy.') });
    renderApp(inFrame(<DevicesPage />));

    await userEvent.click(await screen.findByRole('switch', { name: 'Share Flash drive' }));

    await waitFor(() => expect(isOn(screen.getByRole('switch', { name: 'Share Flash drive' }))).toBe(false));
  });

  it('asks before stopping a device a client has attached, and sends nothing until asked', async () => {
    const { calls } = mockFetch({ ...serverRoutes, 'PUT /api/v1/server/devices/1-1.4/shared': () => accepted() });
    renderApp(inFrame(<DevicesPage />));

    await userEvent.click(await screen.findByRole('switch', { name: 'Share USB Receiver' }));

    const dialog = await screen.findByRole('alertdialog', { name: 'Stop sharing USB Receiver?' });
    expect(calls.some((c) => c.key.startsWith('PUT'))).toBe(false);

    await userEvent.click(within(dialog).getByRole('button', { name: 'Stop sharing' }));

    await waitFor(() => expect(calls.find((c) => c.key === 'PUT /api/v1/server/devices/1-1.4/shared')?.body).toEqual({ shared: false }));
  });

  it('cannot send while the live link is down', async () => {
    mockFetch(serverRoutes);
    renderApp(inFrame(<DevicesPage />, { locked: true }));

    expect(isOff(await screen.findByRole('switch', { name: 'Share Flash drive' }))).toBe(true);
  });
});

// The same bus ID on two servers, free to attach on both.
const lab = clientDevices.servers[0];
const free = (id: string, name: string) => ({
  ...lab,
  id,
  name,
  devices: lab.devices.map((d) => ({ ...d, attachedHere: false, port: null, attachedSince: null })),
});
const twoServers = { ...clientDevices, servers: [free('server-1', 'lab'), free('server-2', 'bench')] };

describe('the devices of every server', () => {
  it('attaches a device by server and bus ID, so the other server keeps its own', async () => {
    const { calls } = mockFetch({ ...baseRoutes, 'GET /api/v1/client/devices': () => json(twoServers), 'PUT /api/v1/client/servers/server-2/devices/1-1.4/attachment': () => accepted() });
    const { live } = renderApp(inFrame(<RemoteDevicesPage />));
    expect(await screen.findByRole('heading', { level: 1, name: 'On lab and bench' })).toBeTruthy();
    const [first, second] = await screen.findAllByRole('switch', { name: 'Attach USB Receiver' });
    expect(isOn(first)).toBe(false);

    await userEvent.click(second);

    await waitFor(() => expect(calls.some((c) => c.key === 'PUT /api/v1/client/servers/server-2/devices/1-1.4/attachment')).toBe(true));
    expect(calls.some((c) => c.key.includes('server-1') && c.key.startsWith('PUT'))).toBe(false);
    const attachedOnBench = { ...twoServers, servers: [twoServers.servers[0], { ...twoServers.servers[1], devices: twoServers.servers[1].devices.map((d) => ({ ...d, attachedHere: true, port: 1 })) }] };
    await act(async () => live.emit('ClientDevicesChanged', attachedOnBench));
    await waitFor(() => expect(isOn(screen.getAllByRole('switch', { name: 'Attach USB Receiver' })[1])).toBe(true));
    expect(isOn(screen.getAllByRole('switch', { name: 'Attach USB Receiver' })[0])).toBe(false);
  });

  it('says which server cannot be reached', async () => {
    const down = { ...twoServers, servers: [{ ...twoServers.servers[0], link: 'error', linkMessage: 'Timed out.', devices: [] }] };
    mockFetch({ ...baseRoutes, 'GET /api/v1/client/devices': () => json(down) });
    renderApp(inFrame(<RemoteDevicesPage />));

    expect(await screen.findByText('Cannot reach lab')).toBeTruthy();
  });

  it('names the usbip command only for a bus ID no earlier server also offers', () => {
    const entries = remoteEntries(twoServers.servers);

    expect(entries.map((e) => [e.server.name, e.unambiguous])).toEqual([
      ['lab', true],
      ['bench', false],
    ]);
  });
});
