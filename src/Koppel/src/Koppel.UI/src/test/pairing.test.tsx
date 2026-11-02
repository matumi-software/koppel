import { act, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DiscoveredServer, PairingStatus } from '@/api/pairing';
import { NearbyServers } from '@/client/nearby-servers';
import { PairingRequests } from '@/server/pairing-requests';
import { accepted, json, mockFetch, renderApp } from './harness';

const fp = (digit: string) => Array.from({ length: 32 }, () => digit.repeat(2)).join(':');
const server = (name: string, digit: string): DiscoveredServer => ({
  name,
  address: '10.0.0.5',
  httpsPort: 5241,
  fingerprint: fp(digit),
  version: '1.0',
  url: `https://10.0.0.5:5241/${digit}`,
});

const idle: PairingStatus = { state: 'idle', serverName: null, serverUrl: null, code: null, message: null, expiresAt: null, serverFingerprint: null };
const noConnection = { serverUrl: null, serverFingerprint: null, clientId: null, hasSecret: false };

const routes = (servers: DiscoveredServer[], connection: unknown = noConnection, status: PairingStatus = idle) => ({
  'GET /api/client/discovery': () => json(servers),
  'GET /api/client/connection': () => json(connection),
  'GET /api/client/pairing': () => json(status),
  'GET /api/activity?max=200': () => json([]),
  'GET /api/system/info': () => json({ name: 'bench-1' }),
});

afterEach(() => vi.unstubAllGlobals());

describe('nearby servers', () => {
  it('shows a short fingerprint on every row', async () => {
    mockFetch(routes([server('lab-a', 'a')]));
    renderApp(<NearbyServers locked={false} />);

    expect(await screen.findByText(/fingerprint AA:AA:AA:AA:AA:AA:AA:AA…/)).toBeTruthy();
  });

  it('warns when two servers share a name with different fingerprints', async () => {
    mockFetch(routes([server('lab', 'a'), server('lab', 'b')]));
    renderApp(<NearbyServers locked={false} />);

    expect((await screen.findAllByText(/same name but a different fingerprint/)).length).toBe(2);
  });

  it('warns when a server wears the name of the current connection with another fingerprint', async () => {
    mockFetch(routes([server('lab-rack-3', 'b')], { serverUrl: 'https://lab-rack-3:5241', serverFingerprint: fp('a'), clientId: 'c', hasSecret: true }));
    renderApp(<NearbyServers locked={false} />);

    expect(await screen.findByText(/matches the server you are connected to, but the fingerprint is different/)).toBeTruthy();
  });

  it('shows no warning for a lone server', async () => {
    mockFetch(routes([server('lab', 'a')]));
    renderApp(<NearbyServers locked={false} />);

    await screen.findByText('lab');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('asks before replacing a connection, and confirms through the API', async () => {
    const { count } = mockFetch({
      ...routes([], noConnection, { ...idle, state: 'confirm', serverName: 'lab', serverUrl: 'https://lab:5241', serverFingerprint: fp('c'), message: 'Saving replaces your connection to https://old:5241.' }),
      'POST /api/client/pairing/confirm': () => accepted(),
    });
    const { hub } = renderApp(<NearbyServers locked={false} />);

    fireEvent.click(await screen.findByText('Replace connection'));
    await vi.waitFor(() => expect(count('POST /api/client/pairing/confirm')).toBe(1));
    act(() => hub.emit('PairingStatusChanged', { ...idle, state: 'paired', serverName: 'lab', serverUrl: 'https://lab:5241', serverFingerprint: fp('c') }));

    await vi.waitFor(() => expect(screen.queryByText('Replace connection')).toBeNull());
  });

  it('shows an error when starting a pairing is refused', async () => {
    mockFetch({
      ...routes([server('lab', 'a')]),
      'POST /api/client/pairing': () => json({ errors: { serverUrl: ['Use the HTTPS address.'] } }, 400),
    });
    renderApp(<NearbyServers locked={false} />);

    fireEvent.click(await screen.findByText('Pair'));

    expect(await screen.findByText(/Not paired: Use the HTTPS address\./)).toBeTruthy();
  });
});

describe('pairing requests', () => {
  it('says why a request was refused', async () => {
    mockFetch({
      'GET /api/server/pairing': () =>
        json([{ id: 'r1', clientName: 'bench-2', from: '10.0.0.9', requestedAt: '2026-01-01T00:00:00Z', expiresAt: '2026-01-01T00:05:00Z', state: 'denied', message: 'The code was wrong three times.' }]),
      'GET /api/activity?max=200': () => json([]),
    });
    renderApp(<PairingRequests locked={false} />);

    expect(await screen.findByText(/Refused bench-2 from 10.0.0.9: The code was wrong three times\./)).toBeTruthy();
  });
});
