import { act, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ClientView } from '@/api/server';
import { ClientsPage } from '@/server/clients-page';
import { SideInfo } from '@/shared/side-info';
import { FrameContext } from '@/shell/frame-context';
import { baseRoutes, frame } from './fixtures';
import { json, mockFetch, renderApp } from './harness';

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('relative times', () => {
  it('move on by themselves: a client last seen just now is seen minutes ago a while later', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date('2026-01-01T10:00:00Z'));
    const client: ClientView = { id: 'bench-1', name: 'Bench 1', createdAt: '2026-01-01T08:00:00Z', hasSecret: true, lastSeen: '2026-01-01T09:59:50Z', hasNextCertificate: false };
    mockFetch({ ...baseRoutes, 'GET /api/v1/server/clients': () => json([client]), 'GET /api/v1/server/keys': () => json({ fingerprint: 'AA', nextFingerprint: null, nextPreparedAt: null, signingKeyId: 'k', signingKeySince: '2026-01-01T00:00:00Z', retiredKeysUntil: null }) });
    renderApp(
      <FrameContext.Provider value={frame()}>
        <ClientsPage />
      </FrameContext.Provider>,
    );
    await screen.findByText('Seen just now');

    await act(() => vi.advanceTimersByTimeAsync(5 * 60_000));

    expect(screen.getByText('Seen 5 min ago')).toBeTruthy();
  });

  it('keep the side panel uptime moving without asking the server again', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date('2026-01-01T01:00:00Z'));
    const { count } = mockFetch(baseRoutes);
    renderApp(<SideInfo />);
    await screen.findByText('1 h 0 min');

    await act(() => vi.advanceTimersByTimeAsync(10 * 60_000));

    expect(screen.getByText('1 h 10 min')).toBeTruthy();
    expect(count('GET /api/v1/system/info')).toBe(1);
  });
});
