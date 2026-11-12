import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useActivity } from '@/activity/activity-context';
import type { KoppelSettings } from '@/api/settings';
import { confirmTimeoutMs } from '@/pending/domains';
import { usePending } from '@/pending/pending-context';
import { useSettings } from '@/settings/use-settings';
import { accepted, deferred, json, mockFetch, renderApp } from './harness';
import { settingsOptions } from '@/queries';

const settings: KoppelSettings = { role: 'server', shareOnStartup: false, setupComplete: true, notifyPairing: true, notifyLink: true, notifyAttachFailures: true };

/** A page that changes one setting and shows what the confirm rule says about it. */
function SettingsPage() {
  const { query, status, requestChange } = useSettings();
  const share = status('shareOnStartup');
  if (!query.data || !share) return <p>Loading</p>;
  return (
    <div>
      <button onClick={() => requestChange('shareOnStartup', true)}>Turn on</button>
      <span data-testid="confirmed">{String(share.confirmed)}</span>
      <span data-testid="pending">{share.pendingTarget === undefined ? 'no' : String(share.pendingTarget)}</span>
      <span data-testid="failure">{share.failure ?? 'none'}</span>
      <span data-testid="saved">{String(share.saved)}</span>
    </div>
  );
}

/** Stays mounted when the page goes: what the rest of the app sees. */
function Watcher() {
  const { pending } = usePending('settings');
  const { items } = useActivity();
  return (
    <div>
      <span data-testid="app-pending">{Object.keys(pending).join(',') || 'none'}</span>
      <span data-testid="app-activity">{items.map((i) => i.text).join('|') || 'none'}</span>
    </div>
  );
}

const text = (id: string) => screen.getByTestId(id).textContent;

const routes = (current: () => KoppelSettings = () => settings) => ({
  'GET /api/settings': () => json(current()),
  'GET /api/activity?max=200': () => json([]),
  'PATCH /api/settings': () => accepted(),
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('the confirm rule', () => {
  it('stays pending until the push brings the value back, then confirms', async () => {
    mockFetch(routes());
    const { live } = renderApp(<SettingsPage />);
    await screen.findByText('Turn on');

    fireEvent.click(screen.getByText('Turn on'));
    await waitFor(() => expect(text('pending')).toBe('true'));
    expect(text('confirmed')).toBe('false');

    await act(async () => live.emit('SettingsChanged', { ...settings, shareOnStartup: true }));

    expect(text('pending')).toBe('no');
    expect(text('confirmed')).toBe('true');
    expect(text('saved')).toBe('true');
  });

  it('keeps waiting when a push is about something else', async () => {
    mockFetch(routes());
    const { live } = renderApp(<SettingsPage />);
    await screen.findByText('Turn on');
    fireEvent.click(screen.getByText('Turn on'));
    await waitFor(() => expect(text('pending')).toBe('true'));

    await act(async () => live.emit('SettingsChanged', { ...settings, role: 'both' }));

    expect(text('pending')).toBe('true');
    expect(text('confirmed')).toBe('false');
  });

  it('fails after the timeout, and a late push with the value clears the failure', async () => {
    mockFetch(routes());
    const { live } = renderApp(<SettingsPage />);
    await screen.findByText('Turn on');
    vi.useFakeTimers();

    fireEvent.click(screen.getByText('Turn on'));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(confirmTimeoutMs + 1);
    });
    expect(text('failure')).toBe('no confirmation from the server.');
    expect(text('pending')).toBe('no');

    await act(async () => live.emit('SettingsChanged', { ...settings, shareOnStartup: true }));

    expect(text('failure')).toBe('none');
    expect(text('confirmed')).toBe('true');
  });

  it('fails at once when the server refuses the request', async () => {
    mockFetch({ ...routes(), 'PATCH /api/settings': () => new Response(null, { status: 500 }) });
    renderApp(<SettingsPage />);
    await screen.findByText('Turn on');

    fireEvent.click(screen.getByText('Turn on'));

    await waitFor(() => expect(text('failure')).toBe('the server answered 500.'));
    expect(text('pending')).toBe('no');
  });

  it('still tracks, logs and settles a change after the page that sent it is gone', async () => {
    mockFetch(routes());
    const { live, rerenderApp } = renderApp(
      <>
        <SettingsPage />
        <Watcher />
      </>,
    );
    await screen.findByText('Turn on');
    fireEvent.click(screen.getByText('Turn on'));
    await waitFor(() => expect(text('app-pending')).toBe('shareOnStartup'));
    expect(text('app-activity')).toContain('Share saved devices on startup: on');

    rerenderApp(<Watcher />);
    expect(text('app-pending')).toBe('shareOnStartup');

    await act(async () => live.emit('SettingsChanged', { ...settings, shareOnStartup: true }));

    expect(text('app-pending')).toBe('none');
    expect(text('app-activity')).toBe('none');
  });

  it('settles a change from the refetch after the link comes back, when the push was missed', async () => {
    let current = settings;
    const { count } = mockFetch(routes(() => current));
    const { queryClient } = renderApp(<SettingsPage />);
    await screen.findByText('Turn on');
    fireEvent.click(screen.getByText('Turn on'));
    await waitFor(() => expect(text('pending')).toBe('true'));

    // The server applied it while the link was down, so no push arrived. LiveProvider invalidates on reconnect.
    current = { ...settings, shareOnStartup: true };
    await act(async () => {
      await queryClient.invalidateQueries();
    });

    expect(count('GET /api/settings')).toBe(2);
    expect(text('pending')).toBe('no');
    expect(text('confirmed')).toBe('true');
    expect(text('saved')).toBe('true');
  });

  it('keeps a push that arrives while an older fetch is still in flight', async () => {
    const slow = deferred<Response>();
    let calls = 0;
    mockFetch({
      ...routes(),
      'GET /api/settings': () => (++calls === 1 ? json(settings) : slow.promise),
    });
    const { live, queryClient } = renderApp(<SettingsPage />);
    await screen.findByText('Turn on');

    // A refetch starts with the old value, then the push arrives, then the old fetch lands.
    void queryClient.invalidateQueries({ queryKey: settingsOptions().queryKey });
    await waitFor(() => expect(calls).toBe(2));
    await act(async () => live.emit('SettingsChanged', { ...settings, shareOnStartup: true }));
    await act(async () => {
      slow.resolve(json(settings));
      await slow.promise;
    });

    await waitFor(() => expect(text('confirmed')).toBe('true'));
    expect(queryClient.getQueryData<KoppelSettings>(settingsOptions().queryKey)?.shareOnStartup).toBe(true);
  });
});

describe('pushes and answers that arrive out of order', () => {
  const event = (id: string, at: string) => ({
    id,
    timestampUtc: at,
    entity: { id: 'x', name: 'x', type: 'device' as const },
    reason: 'shared' as const,
    outcome: 'succeeded' as const,
    summary: id,
    detail: null,
  });

  it('keeps an activity event pushed while the activity fetch is in flight', async () => {
    const slow = deferred<Response>();
    let fetches = 0;
    mockFetch({ ...routes(), 'GET /api/activity?max=200': () => (++fetches === 1 ? json([]) : slow.promise) });
    const { live, queryClient } = renderApp(<Watcher />);
    await waitFor(() => expect(fetches).toBe(1));
    await act(async () => {});

    // The refetch after a reconnect is under way when the push lands, then it answers without it.
    void queryClient.invalidateQueries({ queryKey: ['activity'] });
    await waitFor(() => expect(fetches).toBe(2));
    act(() => live.emit('ActivityRecorded', event('new', '2026-01-01T00:00:02Z')));
    await act(async () => {
      slow.resolve(json([event('old', '2026-01-01T00:00:01Z')]));
      await slow.promise;
    });

    await waitFor(() => expect(text('app-activity')).toBe('new|old'));
    expect(queryClient.getQueryData<{ id: string }[]>(['activity'])?.map((e) => e.id)).toEqual(['new', 'old']);
  });

  it('ignores a rejection that arrives after the change was confirmed', async () => {
    const late = deferred<Response>();
    mockFetch({ ...routes(), 'PATCH /api/settings': () => late.promise });
    const { live } = renderApp(
      <>
        <SettingsPage />
        <Watcher />
      </>,
    );
    await screen.findByText('Turn on');
    fireEvent.click(screen.getByText('Turn on'));
    await waitFor(() => expect(text('pending')).toBe('true'));

    await act(async () => live.emit('SettingsChanged', { ...settings, shareOnStartup: true }));
    await act(async () => {
      late.resolve(new Response(null, { status: 500 }));
      await late.promise;
    });
    // A later push must not trip over a failure that was never recorded.
    await act(async () => live.emit('SettingsChanged', { ...settings, shareOnStartup: false }));

    expect(text('failure')).toBe('none');
    expect(text('saved')).toBe('true');
    expect(text('app-pending')).toBe('none');
  });

  it('does not let a late rejection of an old send fail a newer one', async () => {
    const first = deferred<Response>();
    let sends = 0;
    mockFetch({ ...routes(), 'PATCH /api/settings': () => (++sends === 1 ? first.promise : accepted()) });
    const { live } = renderApp(<SettingsPage />);
    await screen.findByText('Turn on');
    fireEvent.click(screen.getByText('Turn on'));
    await waitFor(() => expect(text('pending')).toBe('true'));
    await act(async () => live.emit('SettingsChanged', { ...settings, shareOnStartup: true }));
    await act(async () => live.emit('SettingsChanged', settings));
    fireEvent.click(screen.getByText('Turn on'));
    await waitFor(() => expect(text('pending')).toBe('true'));

    await act(async () => {
      first.resolve(new Response(null, { status: 500 }));
      await first.promise;
    });

    expect(text('pending')).toBe('true');
    expect(text('failure')).toBe('none');
  });
});

describe('live pushes', () => {
  it('registers one handler per event, however many pages read the cache', async () => {
    mockFetch(routes());
    const { live } = renderApp(
      <>
        <SettingsPage />
        <SettingsPage />
        <Watcher />
      </>,
    );
    await screen.findAllByText('Turn on');

    for (const name of [
      'SettingsChanged',
      'ClientsChanged',
      'ConnectionChanged',
      'DevicesChanged',
      'ClientDevicesChanged',
      'ActivityRecorded',
      'PairingRequestsChanged',
      'PairingStatusChanged',
      'ServersDiscovered',
    ]) {
      expect(live.count(name), name).toBe(1);
    }
  });

  it('merges a recorded activity event into the log by id', async () => {
    const entry = (id: string, at: string) => ({
      id,
      timestampUtc: at,
      entity: { id: 'x', name: 'x', type: 'device' },
      reason: 'shared',
      outcome: 'succeeded',
      summary: `event ${id}`,
      detail: null,
    });
    mockFetch({ ...routes(), 'GET /api/activity?max=200': () => json([entry('a', '2026-01-01T10:00:00Z')]) });
    const { live } = renderApp(<Watcher />);
    await waitFor(() => expect(text('app-activity')).toBe('event a'));

    await act(async () => live.emit('ActivityRecorded', entry('b', '2026-01-01T11:00:00Z')));
    await act(async () => live.emit('ActivityRecorded', entry('b', '2026-01-01T11:00:00Z')));

    await waitFor(() => expect(text('app-activity')).toBe('event b|event a'));
  });
});
