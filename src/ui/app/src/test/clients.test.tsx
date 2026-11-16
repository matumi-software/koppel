import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ClientsPage } from '@/clients/clients-page';
import { lastSeenLine } from '@/clients/client-words';
import { clientsOptions } from '@/queries';
import { accepted, json, mockFetch, newQueryClient, renderApp } from './harness';
import { baseRoutes, inFrame } from './fixtures';
import { clientSecret, clients, enrollmentKey, enrollmentView, newClient, pairingOverview } from './wire';

afterEach(() => vi.unstubAllGlobals());

const routes = {
  ...baseRoutes,
  'GET /api/v1/server/clients': () => json(clients),
  'GET /api/v1/server/enrollment': () => json({ ...enrollmentView, code: null }),
};

describe('how a client was last seen is worded', () => {
  const now = Date.parse('2026-01-01T10:00:00Z');

  it('reads as a sentence in each case', () => {
    expect(lastSeenLine({ lastSeen: null }, now)).toBe('Not seen since Koppel started');
    expect(lastSeenLine({ lastSeen: '2026-01-01T09:59:40Z' }, now)).toBe('Last seen just now');
    expect(lastSeenLine({ lastSeen: '2026-01-01T09:55:00Z' }, now)).toBe('Last seen 5 min ago');
  });
});

describe('the clients of this server', () => {
  it('asks the server again when the page opens, because last seen is not pushed', async () => {
    const stale = clients.map((c) => ({ ...c, lastSeen: null }));
    const fresh = clients.map((c) => ({ ...c, lastSeen: '2026-01-01T08:15:00Z' }));
    mockFetch({ ...routes, 'GET /api/v1/server/clients': () => json(fresh) });
    const queryClient = newQueryClient();
    queryClient.setQueryData(clientsOptions().queryKey, stale);

    renderApp(inFrame(<ClientsPage />), queryClient);

    await waitFor(() => expect(screen.queryByText('Not since Koppel started')).toBeNull());
  });

  it('asks before revoking, and shows Revoking until ClientsChanged drops the client', async () => {
    const { calls } = mockFetch({ ...routes, 'DELETE /api/v1/server/clients/client-2': () => accepted() });
    const { live } = renderApp(inFrame(<ClientsPage />));

    await userEvent.click(await screen.findByRole('button', { name: 'Revoke old-client' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Revoke old-client?' });
    expect(calls.some((c) => c.key.startsWith('DELETE'))).toBe(false);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Revoke' }));

    await screen.findByText('Revoking');
    expect(screen.getByText('old-client')).toBeTruthy();

    await act(async () => live.emit('ClientsChanged', [clients[0]]));

    await waitFor(() => expect(screen.queryByText('old-client')).toBeNull());
    expect(screen.queryByText('Revoking')).toBeNull();
  });

  it('shows the new secret once, after the rotation is confirmed', async () => {
    const { calls } = mockFetch({ ...routes, 'POST /api/v1/server/clients/client-1/secret': () => json(clientSecret) });
    renderApp(inFrame(<ClientsPage />));

    await userEvent.click(await screen.findByRole('button', { name: 'Rotate the secret of bench-laptop' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(calls.some((c) => c.key.startsWith('POST'))).toBe(false);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Rotate secret' }));

    await screen.findByRole('dialog', { name: 'New secret for bench-laptop' });
    expect(calls.find((c) => c.key === 'POST /api/v1/server/clients/client-1/secret')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Done' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('fetches the server key only when asked to show it', async () => {
    const { count } = mockFetch({ ...routes, 'GET /api/v1/server/enrollment/key': () => json(enrollmentKey) });
    renderApp(inFrame(<ClientsPage />));
    const show = await screen.findByRole('button', { name: 'Show' });
    expect(count('GET /api/v1/server/enrollment/key')).toBe(0);

    await userEvent.click(show);

    await screen.findByRole('button', { name: 'Copy server key' });
    expect(count('GET /api/v1/server/enrollment/key')).toBe(1);
  });

  it('shows a new pairing code as pending until a different one is pushed', async () => {
    const { calls } = mockFetch({ ...routes, 'PUT /api/v1/server/enrollment/code': () => accepted() });
    const { live } = renderApp(inFrame(<ClientsPage />));

    await userEvent.click(await screen.findByRole('button', { name: 'Show a new pairing code' }));
    await waitFor(() => expect(calls.some((c) => c.key === 'PUT /api/v1/server/enrollment/code')).toBe(true));
    expect(screen.queryByText('4831 2290')).toBeNull();

    await act(async () => live.emit('EnrollmentChanged', { ...enrollmentView, code: { code: '48312290', expiresAt: new Date(Date.now() + 60_000).toISOString() } }));

    expect(await screen.findByText(/Valid for/)).toBeTruthy();
  });

  it('offers a new code in words once the code has run out', async () => {
    mockFetch({ ...routes, 'GET /api/v1/server/enrollment': () => json({ ...enrollmentView, code: { code: '48312290', expiresAt: new Date(Date.now() - 1000).toISOString() } }) });
    renderApp(inFrame(<ClientsPage />));

    expect(await screen.findByRole('button', { name: 'Show a new code' })).toBeTruthy();
  });

  it('approves a pairing request with the digits typed, and keeps it waiting until it leaves the list', async () => {
    const { calls } = mockFetch({
      ...routes,
      'GET /api/v1/server/pairing': () => json(pairingOverview),
      'POST /api/v1/server/pairing/requests/req-1/approval': () => accepted(),
    });
    const { live } = renderApp(inFrame(<ClientsPage />));
    const field = await screen.findByRole('textbox', { name: 'Code shown on bench-laptop' });
    const approve = screen.getByRole('button', { name: 'Approve' });
    expect((approve as HTMLButtonElement).disabled).toBe(true);

    await userEvent.type(field, '1234 5678');
    await userEvent.click(approve);

    await waitFor(() => expect(calls.find((c) => c.key === 'POST /api/v1/server/pairing/requests/req-1/approval')?.body).toEqual({ code: '12345678' }));
    expect(screen.getByRole('textbox', { name: 'Code shown on bench-laptop' })).toBeTruthy();

    await act(async () => live.emit('PairingRequestsChanged', { ...pairingOverview, requests: [pairingOverview.requests[1]] }));

    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Code shown on bench-laptop' })).toBeNull());
  });

  it('adds a client by hand and shows its ID and secret once', async () => {
    const { calls } = mockFetch({ ...routes, 'POST /api/v1/server/clients': () => json(newClient, 201) });
    renderApp(inFrame(<ClientsPage />));

    await userEvent.click(await screen.findByRole('button', { name: 'Client ID and secret' }));
    await userEvent.type(await screen.findByRole('textbox', { name: 'Name' }), 'new-client');
    await userEvent.click(screen.getByRole('button', { name: 'Add client' }));

    await screen.findByRole('dialog', { name: 'new-client is added' });
    expect(calls.find((c) => c.key === 'POST /api/v1/server/clients')?.body).toEqual({ name: 'new-client' });
    expect(screen.getByText('client-3')).toBeTruthy();
  });
});
