import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ClientView, ServerKeysView } from '@/api/server';
import { ServerKeys } from '@/server/server-keys';
import { accepted, json, mockFetch, renderApp } from './harness';

const fingerprint = 'AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99';
const next = '11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00';
const keys: ServerKeysView = { fingerprint, nextFingerprint: null, nextPreparedAt: null, signingKeyId: 'koppel-1', signingKeySince: '2026-11-03T08:00:00Z', retiredKeysUntil: null };
const client = (name: string, hasNextCertificate: boolean): ClientView => ({
  id: `client-${name}`,
  name,
  createdAt: '2026-01-01T00:00:00Z',
  hasSecret: true,
  lastSeen: null,
  hasNextCertificate,
});

const card = (clients: ClientView[] = []) => <ServerKeys host="bench-pc" httpsPort={5240} clients={clients} locked={false} />;

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('the server certificate and signing key', () => {
  it('rotates the signing key and waits for the push that carries a new key id', async () => {
    const { calls } = mockFetch({ 'GET /api/server/keys': () => json(keys), 'POST /api/server/keys/signing-key/rotate': () => accepted() });
    const { live } = renderApp(card());

    await userEvent.click(await screen.findByRole('button', { name: 'Rotate signing key' }));
    expect(calls.some((c) => c.key === 'POST /api/server/keys/signing-key/rotate')).toBe(true);
    await screen.findByRole('button', { name: 'Rotating' });

    // The same key id again is not the rotation.
    await act(async () => live.emit('ServerKeysChanged', keys));
    expect(screen.getByRole('button', { name: 'Rotating' })).toBeTruthy();

    await act(async () => live.emit('ServerKeysChanged', { ...keys, signingKeyId: 'koppel-0a1b2c3d4e', retiredKeysUntil: '2026-01-01T00:35:00Z' }));
    await screen.findByRole('button', { name: 'Rotate signing key' });
    expect(screen.getByText('koppel-0a1b2c3d4e')).toBeTruthy();
    expect(screen.getByText(/The key it replaced is still accepted until/)).toBeTruthy();
  });

  it('shows the prepared certificate once the push carries it, and who still lacks its fingerprint', async () => {
    mockFetch({ 'GET /api/server/keys': () => json(keys), 'POST /api/server/keys/next-certificate': () => accepted() });
    const { live } = renderApp(card([client('bench-1', true), client('bench-2', false)]));

    await userEvent.click(await screen.findByRole('button', { name: 'Prepare a new certificate' }));
    await screen.findByRole('button', { name: 'Preparing' });
    await act(async () => live.emit('ServerKeysChanged', { ...keys, nextFingerprint: next, nextPreparedAt: '2026-01-01T00:00:00Z' }));

    expect(await screen.findByText(next)).toBeTruthy();
    expect(screen.getByText(/1 of 2 clients have its fingerprint\. Waiting for bench-2/)).toBeTruthy();
  });

  it('warns before a switch that leaves a client behind, and settles on the new fingerprint', async () => {
    const { calls } = mockFetch({
      'GET /api/server/keys': () => json({ ...keys, nextFingerprint: next, nextPreparedAt: '2026-01-01T00:00:00Z' }),
      'POST /api/server/keys/next-certificate/switch': () => accepted(),
    });
    const { live } = renderApp(card([client('bench-1', true), client('bench-2', false)]));

    await userEvent.click(await screen.findByRole('button', { name: 'Switch to it' }));
    expect(await screen.findByText(/bench-2 has not received the new fingerprint and will refuse this server/)).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Switch' }));

    expect(calls.some((c) => c.key === 'POST /api/server/keys/next-certificate/switch')).toBe(true);
    await act(async () => live.emit('ServerKeysChanged', { ...keys, fingerprint: next }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Prepare a new certificate' })).toBeTruthy());
    expect(screen.getByText(next)).toBeTruthy();
  });
});
