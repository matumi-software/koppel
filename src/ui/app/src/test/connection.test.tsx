import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useMemo, useState, type ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ConnectionPage } from '@/connection/connection-page';
import { FrameContext, FrameSlotsContext } from '@/shell/frame-context';
import { accepted, json, mockFetch, renderApp } from './harness';
import { baseRoutes, frame } from './fixtures';
import { discoveredServers, problem, servers, settings, systemInfo } from './wire';

afterEach(() => vi.unstubAllGlobals());

/** The frame a page sits in, with the inspector it hands up drawn beside it, as the shell would. */
function Frame({ children, locked = false }: { children: ReactNode; locked?: boolean }) {
  const [inspector, setInspector] = useState<ReactNode>(null);
  const slots = useMemo(() => ({ setFlush: () => {}, setInspector }), []);
  const api = useMemo(() => frame({ locked }), [locked]);
  return (
    <FrameContext.Provider value={api}>
      <FrameSlotsContext.Provider value={slots}>
        {children}
        <aside>{inspector}</aside>
      </FrameSlotsContext.Provider>
    </FrameContext.Provider>
  );
}

const lab = servers.servers[0];
const asBoth = { ...settings, role: 'both' as const };
const routes = {
  ...baseRoutes,
  'GET /api/v1/settings': () => json(asBoth),
  'PATCH /api/v1/settings': () => accepted(),
  'GET /api/v1/client/discovery': () => json(discoveredServers),
};
const refused = (code: string, status: number, detail: string) => json({ ...problem, code, status, detail }, status);

const field = (name: string) => screen.findByRole('textbox', { name: new RegExp(name) });

describe('adding a server', () => {
  it('fills the address from a server found on the network', async () => {
    mockFetch({ ...routes, 'GET /api/v1/client/servers': () => json({ servers: [] }) });
    renderApp(<Frame><ConnectionPage /></Frame>);

    await userEvent.click(await screen.findByRole('button', { name: /lab-rack-3/ }));

    expect(((await field('Server address')) as HTMLInputElement).value).toBe('192.168.1.20:5241');
  });

  it('sends the address with the code, and shows why the server refused it under the code', async () => {
    const { calls } = mockFetch({ ...routes, 'GET /api/v1/client/servers': () => json({ servers: [] }), 'POST /api/v1/client/servers': () => refused('enrolment_failed', 422, 'The server did not accept that code.') });
    renderApp(<Frame><ConnectionPage /></Frame>);
    await userEvent.type(await field('Server address'), 'lab:5241');
    await userEvent.type(await field('Pairing code'), '1234 5678');

    await userEvent.click(screen.getByRole('button', { name: 'Connect' }));

    expect(await screen.findByText('The server did not accept that code.')).toBeTruthy();
    expect(calls.find((c) => c.key === 'POST /api/v1/client/servers')?.body).toEqual({ address: 'lab:5241', code: '12345678' });
  });

  it('says a server is added already under the address, and sends the key when asked to', async () => {
    const { calls } = mockFetch({ ...routes, 'GET /api/v1/client/servers': () => json({ servers: [] }), 'POST /api/v1/client/servers': () => refused('already_added', 409, 'That server is added already.') });
    renderApp(<Frame><ConnectionPage /></Frame>);
    await userEvent.click(await screen.findByRole('button', { name: 'Use the server key' }));
    await userEvent.type(await field('Server address'), 'lab:5241');
    await userEvent.type(await field('Server key'), 'kp1_abc');

    await userEvent.click(screen.getByRole('button', { name: 'Connect' }));

    expect(await screen.findByText('That server is added already.')).toBeTruthy();
    expect(calls.find((c) => c.key === 'POST /api/v1/client/servers')?.body).toEqual({ address: 'lab:5241', key: 'kp1_abc' });
  });

  it('shows the added server once the push brings it, and closes the form on the 201', async () => {
    mockFetch({ ...routes, 'GET /api/v1/client/servers': () => json({ servers: [lab] }), 'POST /api/v1/client/servers': () => json({ ...lab, id: 'server-2', name: 'bench' }, 201) });
    const { live } = renderApp(<Frame><ConnectionPage /></Frame>);
    await userEvent.click(await screen.findByRole('button', { name: 'Add a server' }));
    await userEvent.type(await field('Server address'), 'bench:5241');
    await userEvent.type(await field('Pairing code'), '12345678');

    await userEvent.click(screen.getByRole('button', { name: 'Add server' }));

    await waitFor(() => expect(screen.queryByRole('button', { name: 'Add server' })).toBeNull());
    await act(async () => live.emit('ServersChanged', { servers: [lab, { ...lab, id: 'server-2', name: 'bench' }] }));
    expect(await screen.findByRole('button', { name: 'Remove bench' })).toBeTruthy();
  });
});

describe('the staged settings of a server', () => {
  it('sends nothing until Apply, then stays pending until ServersChanged shows every field', async () => {
    const { calls } = mockFetch({ ...routes, 'GET /api/v1/client/servers': () => json(servers), 'PATCH /api/v1/client/servers/server-1': () => accepted() });
    const { live } = renderApp(<Frame><ConnectionPage /></Frame>);
    const reconnect = await screen.findByRole('switch', { name: 'Reconnect' });
    const apply = screen.getByRole('button', { name: 'Apply' });
    expect((apply as HTMLButtonElement).disabled).toBe(true);

    await userEvent.click(reconnect);

    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(false);
    expect(calls.some((c) => c.key.startsWith('PATCH'))).toBe(false);
    await userEvent.click(screen.getByRole('button', { name: 'Apply' }));

    await waitFor(() => expect(calls.find((c) => c.key === 'PATCH /api/v1/client/servers/server-1')?.body).toEqual({ reconnect: false }));
    await screen.findByText('Applying');

    await act(async () => live.emit('ServersChanged', { servers: [{ ...lab, reconnect: false }] }));

    await waitFor(() => expect(screen.queryByText('Applying')).toBeNull());
  });

  it('puts the edits back on Revert without sending anything', async () => {
    const { calls } = mockFetch({ ...routes, 'GET /api/v1/client/servers': () => json(servers) });
    renderApp(<Frame><ConnectionPage /></Frame>);
    await userEvent.click(await screen.findByRole('switch', { name: 'Reconnect' }));

    await userEvent.click(screen.getByRole('button', { name: 'Revert' }));

    expect((screen.getByRole('switch', { name: 'Reconnect' }) as HTMLInputElement).checked).toBe(true);
    expect(calls.some((c) => c.key.startsWith('PATCH'))).toBe(false);
  });

  it('removes a server only after asking, and shows Removing until it leaves the list', async () => {
    const { calls } = mockFetch({ ...routes, 'GET /api/v1/client/servers': () => json(servers), 'DELETE /api/v1/client/servers/server-1': () => accepted() });
    const { live } = renderApp(<Frame><ConnectionPage /></Frame>);

    await userEvent.click(await screen.findByRole('button', { name: 'Remove lab' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Remove lab?' });
    expect(calls.some((c) => c.key.startsWith('DELETE'))).toBe(false);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Remove' }));

    await screen.findByText('Removing');

    await act(async () => live.emit('ServersChanged', { servers: [] }));

    expect(await screen.findByText('Pair with a server')).toBeTruthy();
  });
});

describe('this server', () => {
  const serverOnly = { ...routes, 'GET /api/v1/settings': () => json({ ...settings, role: 'server', networkPort: 6000 }) };

  it('says a saved port applies after Koppel restarts', async () => {
    mockFetch(serverOnly);
    renderApp(<Frame><ConnectionPage /></Frame>);

    expect(await screen.findByText('Applies after Koppel restarts.')).toBeTruthy();
  });

  it('shows the port read-only with the reason when a startup option sets it', async () => {
    mockFetch({ ...serverOnly, 'GET /api/v1/system/info': () => json({ ...systemInfo, networkPortFixed: true }) });
    renderApp(<Frame><ConnectionPage /></Frame>);

    expect(await screen.findByText('A startup option sets the port, so the saved one is not used.')).toBeTruthy();
  });

  it('applies Accept from at once, without Apply', async () => {
    const { calls } = mockFetch(serverOnly);
    renderApp(<Frame><ConnectionPage /></Frame>);
    await userEvent.click(await screen.findByRole('button', { name: /Accept from/ }));

    await userEvent.click(await screen.findByRole('option', { name: 'This network only' }));

    await waitFor(() => expect(calls.find((c) => c.key === 'PATCH /api/v1/settings')?.body).toEqual({ networkAcceptFrom: 'lan' }));
  });

  it('sends the port only on Apply', async () => {
    const { calls } = mockFetch(serverOnly);
    renderApp(<Frame><ConnectionPage /></Frame>);
    const port = await screen.findByRole('textbox', { name: /Port/ });

    await userEvent.clear(port);
    await userEvent.type(port, '6100');
    await userEvent.tab();
    expect(calls.some((c) => c.key.startsWith('PATCH'))).toBe(false);
    await userEvent.click(screen.getByRole('button', { name: 'Apply' }));

    await waitFor(() => expect(calls.find((c) => c.key === 'PATCH /api/v1/settings')?.body).toEqual({ networkPort: 6100 }));
  });
});
