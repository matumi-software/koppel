import type { QueryClient } from '@tanstack/react-query';
import type { ActivityApi } from '@/activity/activity-context';

/** What confirmed data says about one pending change. */
export type Verdict = 'confirmed' | 'waiting' | { failed: string };

/** How one kind of change is judged: against which cached data, and how it reads in the activity log. */
export interface PendingDomain<T, D> {
  queryKey: readonly unknown[];
  /** How long to wait for the server's confirmation before calling a change failed. */
  timeoutMs: number;
  /** How the change reads in the activity log, such as "Share bus 1-2". */
  describe?: (key: string, target: T, queryClient: QueryClient) => { text: string; subject?: string };
  verdict: (key: string, target: T, data: D) => Verdict;
}

export interface PendingSnapshot<T> {
  /** Changes sent and not yet confirmed, by item. */
  pending: Readonly<Record<string, T>>;
  failures: Readonly<Record<string, string>>;
  /** How many changes of each item the server has confirmed since the page loaded. */
  saved: Readonly<Record<string, number>>;
}

/**
 * Pending changes of one domain, for the confirm rule: a change is pending from the moment it is
 * sent until confirmed data says so, a failure says otherwise, or the timeout runs out. Each change
 * is also logged. It lives above the pages, so leaving a page orphans nothing, and it is judged
 * whenever the cache holds new confirmed data, whether that came from a push or a refetch.
 */
export class PendingStore<T, D> {
  private snapshot: PendingSnapshot<T> = { pending: {}, failures: {}, saved: {} };
  private readonly listeners = new Set<() => void>();
  private readonly failedTargets = new Map<string, T>();
  private readonly logIds = new Map<string, string>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();

  private readonly domain: PendingDomain<T, D>;
  private readonly queryClient: QueryClient;
  private activityApi?: ActivityApi;

  constructor(domain: PendingDomain<T, D>, queryClient: QueryClient) {
    this.domain = domain;
    this.queryClient = queryClient;
  }

  /** The activity log to write to. The provider hands over the current one whenever it changes. */
  attachActivity(api: ActivityApi) {
    this.activityApi = api;
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };

  getSnapshot = () => this.snapshot;

  private publish(next: Partial<PendingSnapshot<T>>) {
    this.snapshot = { ...this.snapshot, ...next };
    this.listeners.forEach((listener) => listener());
  }

  private without<V>(record: Readonly<Record<string, V>>, key: string) {
    const next = { ...record };
    delete next[key];
    return next;
  }

  /** `serverLogged`: the server's own activity log already has the outcome, so our line goes. */
  private settle(key: string, failure?: string, serverLogged = false) {
    clearTimeout(this.timers.get(key));
    this.timers.delete(key);
    const target = this.snapshot.pending[key];
    if (failure) this.failedTargets.set(key, target);
    else this.failedTargets.delete(key);
    this.publish({
      pending: this.without(this.snapshot.pending, key),
      failures: failure ? { ...this.snapshot.failures, [key]: failure } : this.without(this.snapshot.failures, key),
      saved: failure ? this.snapshot.saved : this.confirmed(key),
    });
    const logId = this.logIds.get(key);
    if (logId) {
      this.logIds.delete(key);
      if (failure && !serverLogged) this.activityApi?.update(logId, { state: 'fault', lead: 'Not done.', detail: failure });
      else this.activityApi?.remove(logId);
    }
  }

  private confirmed(key: string) {
    return { ...this.snapshot.saved, [key]: (this.snapshot.saved[key] ?? 0) + 1 };
  }

  /** Marks the change pending and sends it. A refused request fails it at once. */
  request = (key: string, target: T, send: () => Promise<void>) => {
    if (key in this.snapshot.pending) return;
    this.failedTargets.delete(key);
    this.publish({ pending: { ...this.snapshot.pending, [key]: target }, failures: this.without(this.snapshot.failures, key) });
    const describe = this.domain.describe?.(key, target, this.queryClient);
    const logId = describe && this.activityApi?.add({ state: 'pending', lead: 'Waiting.', text: describe.text, subject: describe?.subject });
    if (logId) this.logIds.set(key, logId);
    this.timers.set(key, setTimeout(() => this.settle(key, 'no confirmation from the server.'), this.domain.timeoutMs));
    // The send may throw before it returns a promise.
    Promise.resolve()
      .then(send)
      .catch((error: unknown) => this.settle(key, error instanceof Error ? error.message : 'the request failed.'));
  };

  /**
   * Call whenever the cache holds new confirmed data. Settles each pending change the data decides,
   * and clears a failure once the data shows the value that failed after all.
   */
  judge(data: D) {
    for (const [key, target] of Object.entries(this.snapshot.pending)) {
      const result = this.domain.verdict(key, target, data);
      if (result === 'confirmed') this.settle(key);
      else if (result !== 'waiting') this.settle(key, result.failed, true);
    }
    for (const [key, target] of this.failedTargets) {
      if (this.domain.verdict(key, target, data) === 'confirmed') {
        this.failedTargets.delete(key);
        this.publish({ failures: this.without(this.snapshot.failures, key), saved: this.confirmed(key) });
      }
    }
  }

  dispose() {
    this.timers.forEach(clearTimeout);
    this.timers.clear();
  }
}
