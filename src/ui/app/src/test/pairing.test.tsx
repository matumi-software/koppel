import { act, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useActivity } from '@/activity/activity-context';
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
  otherAddresses: [],
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

function ActivityLines() {
  const { items } = useActivity();
  return <span data-testid="activity">{items.map((i) => `${i.lead ?? ''} ${i.text}`.trim()).join('|')}</span>;
}

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

  it('warns when other addresses answer with the same fingerprint', async () => {
    mockFetch(routes([{ ...server('lab', 'a'), otherAddresses: ['10.0.0.7', '10.0.0.8:6000'] }]));
    renderApp(<NearbyServers locked={false} />);

    expect(await screen.findByText(/10\.0\.0\.7, 10\.0\.0\.8:6000 also answer with this fingerprint/)).toBeTruthy();
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
        json({ budgetExhausted: false, requests: [{ id: 'r1', clientName: 'bench-2', from: '10.0.0.9', requestedAt: '2026-01-01T00:00:00Z', expiresAt: '2026-01-01T00:05:00Z', state: 'denied', message: 'The code was wrong three times.' }] }),
      'GET /api/activity?max=200': () => json([]),
    });
    renderApp(<PairingRequests locked={false} />);

    expect(await screen.findByText(/Refused bench-2 from 10.0.0.9: The code was wrong three times\./)).toBeTruthy();
  });

  it('tells the admin when the server is turning pairing requests away', async () => {
    mockFetch({
      'GET /api/server/pairing': () => json({ budgetExhausted: true, requests: [] }),
      'GET /api/activity?max=200': () => json([]),
    });
    renderApp(<PairingRequests locked={false} />);

    expect(await screen.findByText(/too many pairing requests and is turning new ones away/)).toBeTruthy();
  });

  it('shows the alert when a push says the budget ran out, and drops it when it returns', async () => {
    mockFetch({
      'GET /api/server/pairing': () => json({ budgetExhausted: false, requests: [] }),
      'GET /api/activity?max=200': () => json([]),
    });
    const { hub } = renderApp(<PairingRequests locked={false} />);
    await new Promise((resolve) => setTimeout(resolve, 50));

    await act(async () => hub.emit('PairingRequestsChanged', { budgetExhausted: true, requests: [] }));
    expect(await screen.findByText(/turning new ones away/)).toBeTruthy();

    await act(async () => hub.emit('PairingRequestsChanged', { budgetExhausted: false, requests: [] }));
    await vi.waitFor(() => expect(screen.queryByText(/turning new ones away/)).toBeNull());
  });

  it('shows nothing when there is nothing to say', async () => {
    mockFetch({
      'GET /api/server/pairing': () => json({ budgetExhausted: false, requests: [] }),
      'GET /api/activity?max=200': () => json([]),
    });
    const { container } = renderApp(<PairingRequests locked={false} />);

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(container.textContent).toBe('');
  });
});

describe('approving and refusing a request', () => {
  const waiting = { id: 'r1', clientName: 'bench-2', from: '10.0.0.9', requestedAt: '2026-01-01T00:00:00Z', expiresAt: '2026-01-01T00:05:00Z', state: 'waiting', message: null };
  const activityText = () => screen.getByTestId('activity').textContent;

  const open = () =>
    renderApp(
      <>
        <PairingRequests locked={false} />
        <ActivityLines />
      </>,
    );

  it('is pending with an activity line until the request leaves the list', async () => {
    const { count } = mockFetch({
      'GET /api/server/pairing': () => json({ budgetExhausted: false, requests: [waiting] }),
      'GET /api/activity?max=200': () => json([]),
      'POST /api/server/pairing/r1/approve': () => accepted(),
    });
    const { hub } = open();
    fireEvent.change(await screen.findByLabelText(/bench-2 from 10.0.0.9/), { target: { value: '123456' } });

    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));

    await vi.waitFor(() => expect(count('POST /api/server/pairing/r1/approve')).toBe(1));
    expect(activityText()).toContain('Waiting. Approve pairing with bench-2');
    expect((screen.getByRole('button', { name: 'Refuse' }) as HTMLButtonElement).disabled).toBe(true);

    await act(async () => hub.emit('PairingRequestsChanged', { budgetExhausted: false, requests: [] }));

    await vi.waitFor(() => expect(screen.queryByText(/bench-2 from/)).toBeNull());
    expect(activityText()).toBe('');
  });

  it('shows the refusal of a wrong code, and the activity line says it was not done', async () => {
    mockFetch({
      'GET /api/server/pairing': () => json({ budgetExhausted: false, requests: [waiting] }),
      'GET /api/activity?max=200': () => json([]),
      'POST /api/server/pairing/r1/approve': () => json({ errors: { code: ['That code is not right.'] } }, 400),
    });
    open();
    fireEvent.change(await screen.findByLabelText(/bench-2 from 10.0.0.9/), { target: { value: '123456' } });

    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));

    expect(await screen.findByText('Not done: That code is not right.')).toBeTruthy();
    expect(activityText()).toContain('Not done. Approve pairing with bench-2');
  });

  it('refuses through the same store', async () => {
    const { count } = mockFetch({
      'GET /api/server/pairing': () => json({ budgetExhausted: false, requests: [waiting] }),
      'GET /api/activity?max=200': () => json([]),
      'POST /api/server/pairing/r1/deny': () => accepted(),
    });
    open();

    fireEvent.click(await screen.findByRole('button', { name: 'Refuse' }));

    await vi.waitFor(() => expect(count('POST /api/server/pairing/r1/deny')).toBe(1));
    expect(activityText()).toContain('Refuse pairing with bench-2');
  });
});

describe('pairing with a nearby server', () => {
  it('counts the pairing as under way when the server spells the address its own way', async () => {
    const { count } = mockFetch({ ...routes([server('lab', 'a')]), 'POST /api/client/pairing': () => accepted() });
    const { hub } = renderApp(
      <>
        <NearbyServers locked={false} />
        <ActivityLines />
      </>,
    );
    fireEvent.click(await screen.findByRole('button', { name: 'Pair' }));
    await vi.waitFor(() => expect(count('POST /api/client/pairing')).toBe(1));
    expect(screen.getByTestId('activity').textContent).toContain('Pair with lab');

    act(() => hub.emit('PairingStatusChanged', { ...idle, state: 'waiting', serverName: 'lab', serverUrl: 'HTTPS://10.0.0.5:5241', code: '123456' }));

    await vi.waitFor(() => expect(screen.getByTestId('activity').textContent).toBe(''));
  });
});
