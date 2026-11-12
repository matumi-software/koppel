import { useQuery } from '@tanstack/react-query';
import { act, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { getSettings, type KoppelSettings } from '@/api/settings';
import { useLive } from '@/live/live-context';
import { downAfterFailures, pushEvents, retryDelay } from '@/live/push-table';
import { settingsKey } from '@/query-keys';
import { FakeEventSource } from './fake-event-source';
import { json, mockFetch, renderApp } from './harness';

const settings: KoppelSettings = { role: 'server', shareOnStartup: false, setupComplete: true, notifyPairing: true, notifyLink: true, notifyAttachFailures: true };

/** Shows the link's state and the settings the cache holds. */
function Probe() {
  const { state } = useLive();
  const query = useQuery({ queryKey: settingsKey, queryFn: getSettings });
  return (
    <div>
      <span data-testid="state">{state}</span>
      <span data-testid="role">{query.data?.role ?? 'none'}</span>
    </div>
  );
}

const state = () => screen.getByTestId('state').textContent;

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('the live link', () => {
  it('reconnects on the same steps as before, for as long as it takes', () => {
    expect([0, 1, 2, 3, 4].map(retryDelay)).toEqual([0, 2000, 5000, 10000, 30000]);
    expect(retryDelay(9)).toBe(30000);
  });

  it('listens once for each event in the push table, on one stream', async () => {
    mockFetch({ 'GET /api/settings': () => json(settings) });
    renderApp(<Probe />);
    await waitFor(() => expect(state()).toBe('live'));

    expect(FakeEventSource.instances.length).toBe(1);
    for (const name of pushEvents) expect(FakeEventSource.current.count(name), name).toBe(1);
  });

  it('does not fetch again when the first stream opens', async () => {
    const { count } = mockFetch({ 'GET /api/settings': () => json(settings) });
    renderApp(<Probe />);
    await waitFor(() => expect(state()).toBe('live'));
    await screen.findByText('server');

    expect(count('GET /api/settings')).toBe(1);
  });

  it('keeps trying after a first connect that failed, and fetches everything once it is back', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    FakeEventSource.answering = false;
    const { count } = mockFetch({ 'GET /api/settings': () => json(settings) });
    renderApp(<Probe />);
    await waitFor(() => expect(FakeEventSource.instances.length).toBe(1));
    expect(state()).toBe('reconnecting');

    await act(() => vi.advanceTimersByTimeAsync(retryDelay(0)));
    expect(FakeEventSource.instances.length).toBe(2);
    await act(() => vi.advanceTimersByTimeAsync(retryDelay(1)));
    expect(FakeEventSource.instances.length).toBe(3);

    FakeEventSource.answering = true;
    await act(() => vi.advanceTimersByTimeAsync(retryDelay(2)));

    await waitFor(() => expect(state()).toBe('live'));
    await waitFor(() => expect(count('GET /api/settings')).toBe(2));
  });

  it('calls the link down once it has been away through every step, and goes on trying', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    FakeEventSource.answering = false;
    mockFetch({ 'GET /api/settings': () => json(settings) });
    renderApp(<Probe />);
    await waitFor(() => expect(FakeEventSource.instances.length).toBe(1));

    for (let attempt = 0; attempt < downAfterFailures; attempt++) {
      expect(state()).toBe('reconnecting');
      await act(() => vi.advanceTimersByTimeAsync(retryDelay(attempt)));
    }

    expect(state()).toBe('down');
    const before = FakeEventSource.instances.length;
    await act(() => vi.advanceTimersByTimeAsync(retryDelay(downAfterFailures)));
    expect(FakeEventSource.instances.length).toBe(before + 1);
  });
});

describe('a push', () => {
  it('lands in the cache, and the later of two back to back pushes wins', async () => {
    mockFetch({ 'GET /api/settings': () => json(settings) });
    const { queryClient, live } = renderApp(<Probe />);
    await screen.findByText('server');

    act(() => {
      live.emit('SettingsChanged', { ...settings, role: 'client' });
      live.emit('SettingsChanged', { ...settings, role: 'both' });
    });

    expect(queryClient.getQueryData<KoppelSettings>(settingsKey)?.role).toBe('both');
  });

  it('that is malformed is logged and its data fetched again', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    let current = settings;
    const { count } = mockFetch({ 'GET /api/settings': () => json(current) });
    const { live } = renderApp(<Probe />);
    await screen.findByText('server');

    current = { ...settings, role: 'client' };
    await act(async () => live.emitRaw('SettingsChanged', '{not json'));

    await screen.findByText('client');
    expect(count('GET /api/settings')).toBe(2);
    expect(error).toHaveBeenCalledOnce();
  });

  it('that the cache entry cannot take is logged and its data fetched again', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { count } = mockFetch({ 'GET /api/settings': () => json(settings), 'GET /api/activity?max=200': () => json([]) });
    const { live } = renderApp(<Probe />);
    await screen.findByText('server');

    await act(async () => live.emitRaw('ActivityRecorded', 'null'));

    await waitFor(() => expect(count('GET /api/activity?max=200')).toBe(2));
    expect(error).toHaveBeenCalledOnce();
  });
});
