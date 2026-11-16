import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { addServer } from '@/api/servers';
import { RejectedError } from '@/api/http';
import { useEnrollmentCode } from '@/clients/use-enrollment';
import { useServerChanges, useServers } from '@/connection/use-servers';
import { usePending } from '@/pending/pending-context';
import { attachmentKey } from '@/pending/domains';
import { accepted, invalid, json, mockFetch, renderApp } from './harness';
import { clientDevices, enrollmentView, servers } from './wire';

afterEach(() => vi.unstubAllGlobals());

const lab = servers.servers[0];

function ServerRow() {
  const { data } = useServers();
  const { pending, failures, change, remove } = useServerChanges();
  const server = data?.servers[0];
  if (!server) return <p>Loading</p>;
  return (
    <div>
      <span data-testid="retry">{server.retryEverySeconds}</span>
      <span data-testid="pending">{Object.keys(pending).join()}</span>
      <span data-testid="failure">{failures[server.id] ?? ''}</span>
      <button onClick={() => change(server, { retryEverySeconds: 10, reconnect: false })}>Apply</button>
      <button onClick={() => remove(server)}>Remove</button>
    </div>
  );
}

describe('servers', () => {
  it('stays pending until ServersChanged shows every changed field', async () => {
    const { calls } = mockFetch({ 'GET /api/v1/client/servers': () => json(servers), 'PATCH /api/v1/client/servers/server-1': () => accepted() });
    const { live } = renderApp(<ServerRow />);
    await userEvent.click(await screen.findByText('Apply'));

    expect(calls.find((c) => c.key === 'PATCH /api/v1/client/servers/server-1')?.body).toEqual({ retryEverySeconds: 10, reconnect: false });
    await waitFor(() => expect(screen.getByTestId('pending').textContent).toBe('server-1'));

    // Only one of the two fields has landed: not confirmed.
    await act(async () => live.emit('ServersChanged', { servers: [{ ...lab, retryEverySeconds: 10 }] }));
    expect(screen.getByTestId('pending').textContent).toBe('server-1');

    await act(async () => live.emit('ServersChanged', { servers: [{ ...lab, retryEverySeconds: 10, reconnect: false }] }));
    await waitFor(() => expect(screen.getByTestId('pending').textContent).toBe(''));
    expect(screen.getByTestId('retry').textContent).toBe('10');
  });

  it('settles a removal when the server is gone from the list', async () => {
    const { calls } = mockFetch({ 'GET /api/v1/client/servers': () => json(servers), 'DELETE /api/v1/client/servers/server-1': () => accepted() });
    const { live } = renderApp(<ServerRow />);
    await userEvent.click(await screen.findByText('Remove'));
    await waitFor(() => expect(calls.some((c) => c.key === 'DELETE /api/v1/client/servers/server-1')).toBe(true));
    expect(screen.getByTestId('pending').textContent).toBe('server-1');

    await act(async () => live.emit('ServersChanged', { servers: [] }));

    await screen.findByText('Loading');
  });

  it('carries the problem of a refused add, with its code and the message per field', async () => {
    mockFetch({
      'POST /api/v1/client/servers': () => invalid('code', 'The pairing code is 8 digits.'),
    });

    const error = await addServer({ address: 'lab:5241', code: '12' }).catch((e: unknown) => e);

    expect(error).toBeInstanceOf(RejectedError);
    expect((error as RejectedError).message).toBe('The pairing code is 8 digits.');
    expect((error as RejectedError).problem?.errors).toEqual({ code: ['The pairing code is 8 digits.'] });
  });
});

function Code() {
  const { code, open, close, isPending } = useEnrollmentCode();
  return (
    <div>
      <span data-testid="code">{code?.code ?? 'none'}</span>
      <span data-testid="busy">{String(isPending)}</span>
      <button onClick={open}>Show</button>
      <button onClick={close}>Close</button>
    </div>
  );
}

describe('the pairing code', () => {
  it('is pending until a different code shows, then until none does', async () => {
    mockFetch({
      'GET /api/v1/server/enrollment': () => json(enrollmentView),
      'PUT /api/v1/server/enrollment/code': () => accepted(),
      'DELETE /api/v1/server/enrollment/code': () => accepted(),
    });
    const { live } = renderApp(<Code />);
    await waitFor(() => expect(screen.getByTestId('code').textContent).toBe('48312290'));

    await userEvent.click(screen.getByText('Show'));
    await waitFor(() => expect(screen.getByTestId('busy').textContent).toBe('true'));

    // The old code again is not an answer to "show a new one".
    await act(async () => live.emit('EnrollmentChanged', enrollmentView));
    expect(screen.getByTestId('busy').textContent).toBe('true');

    await act(async () => live.emit('EnrollmentChanged', { ...enrollmentView, code: { code: '11112222', expiresAt: '2026-01-01T08:25:00Z' } }));
    await waitFor(() => expect(screen.getByTestId('busy').textContent).toBe('false'));

    await userEvent.click(screen.getByText('Close'));
    await act(async () => live.emit('EnrollmentChanged', { ...enrollmentView, code: null }));
    await waitFor(() => expect(screen.getByTestId('code').textContent).toBe('none'));
    expect(screen.getByTestId('busy').textContent).toBe('false');
  });
});

function Attach() {
  const { request, pending } = usePending('clientDevices');
  const key = attachmentKey('server-1', '1-1.4');
  return (
    <div>
      <span data-testid="pending">{Object.keys(pending).join()}</span>
      <button onClick={() => request({ serverId: 'server-1', busId: '1-1.4', attach: false })}>Detach</button>
      <span>{key}</span>
    </div>
  );
}

describe('a device on a server', () => {
  it('is named by server and bus ID, and settled by that server entry only', async () => {
    const { calls } = mockFetch({ 'DELETE /api/v1/client/servers/server-1/devices/1-1.4/attachment': () => accepted() });
    const { live } = renderApp(<Attach />);
    await userEvent.click(screen.getByText('Detach'));
    await waitFor(() => expect(calls.filter((c) => c.key.startsWith('DELETE'))).toHaveLength(1));
    expect(screen.getByTestId('pending').textContent).toBe('server-1/1-1.4');

    // The same bus ID on another server still attached here does not settle it.
    const other = { ...clientDevices.servers[0], id: 'server-2' };
    await act(async () => live.emit('ClientDevicesChanged', { ...clientDevices, servers: [other, { ...clientDevices.servers[0], devices: clientDevices.servers[0].devices }] }));
    expect(screen.getByTestId('pending').textContent).toBe('server-1/1-1.4');

    await act(async () =>
      live.emit('ClientDevicesChanged', {
        ...clientDevices,
        servers: [{ ...clientDevices.servers[0], devices: clientDevices.servers[0].devices.map((d) => ({ ...d, attachedHere: false })) }],
      }),
    );
    await waitFor(() => expect(screen.getByTestId('pending').textContent).toBe(''));
  });
});
