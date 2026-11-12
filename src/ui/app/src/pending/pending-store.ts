import type { ActivityApi } from '@/activity/activity-context';

/** What confirmed data says about one pending change. */
export type Verdict = 'confirmed' | 'waiting' | { failed: string };

/**
 * One kind of change the UI sends: what it asks for (`A`, a union with a `type` where there is more
 * than one change), the confirmed data that settles it (`D`), and what the send needs besides the
 * action (`I`, such as a secret that must not stay in the store).
 */
export interface PendingDomain<A, D, I = void> {
  queryKey: readonly unknown[];
  /** How long to wait for the server's confirmation before calling a change failed. */
  timeoutMs: number;
  /** The item a change is about, such as a bus ID: one change per item is pending at a time. */
  keyOf: (action: A) => string;
  /** How the change reads in the activity log, such as "Share bus 1-2". */
  describe?: (action: A) => { text: string; subject?: string };
  verdict: (action: A, data: D) => Verdict;
  /** The request. It resolves when the server accepted the change, which is not a confirmation. */
  send: (action: A, input: I) => Promise<void>;
}

export interface PendingSnapshot<A> {
  /** Changes sent and not yet confirmed, by item. */
  pending: Readonly<Record<string, A>>;
  failures: Readonly<Record<string, string>>;
  /** How many changes of each item the server has confirmed since the page loaded. */
  saved: Readonly<Record<string, number>>;
  /** Like `saved`, but counts only changes that were pending when confirmed, not a late confirm of one that had failed. */
  completed: Readonly<Record<string, number>>;
}

/**
 * Pending changes of one domain, for the confirm rule: a change is pending from the moment it is
 * sent until confirmed data says so, a failure says otherwise, or the timeout runs out. Each change
 * is also logged. It lives above the pages, so leaving a page orphans nothing, and it is judged
 * whenever the cache holds new confirmed data, whether that came from a push or a refetch. The send
 * itself is a mutation in usePending; the store only learns that it began and that it failed.
 */
export class PendingStore<A, D, I = void> {
  private snapshot: PendingSnapshot<A> = { pending: {}, failures: {}, saved: {}, completed: {} };
  private readonly listeners = new Set<() => void>();
  private readonly failedActions = new Map<string, A>();
  private readonly logIds = new Map<string, string>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  /** Which send each pending key is waiting on, so a late answer to an old send cannot settle a newer one. */
  private readonly sends = new Map<string, number>();
  private sendCount = 0;
  private activityApi?: ActivityApi;
  private announcer?: (message: string) => void;

  readonly domain: PendingDomain<A, D, I>;

  constructor(domain: PendingDomain<A, D, I>) {
    this.domain = domain;
  }

  /** The activity log to write to. The provider hands over the current one whenever it changes. */
  attachActivity(api: ActivityApi) {
    this.activityApi = api;
  }

  /** Where to say how a change ended, so a screen reader hears it: the app's one live region. */
  attachAnnouncer(announce: (message: string) => void) {
    this.announcer = announce;
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };

  getSnapshot = () => this.snapshot;

  private publish(next: Partial<PendingSnapshot<A>>) {
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
    // Already settled, for example a rejection that arrives after the confirmation.
    if (!(key in this.snapshot.pending)) return;
    this.sends.delete(key);
    clearTimeout(this.timers.get(key));
    this.timers.delete(key);
    const action = this.snapshot.pending[key];
    if (failure) this.failedActions.set(key, action);
    else this.failedActions.delete(key);
    this.publish({
      pending: this.without(this.snapshot.pending, key),
      failures: failure ? { ...this.snapshot.failures, [key]: failure } : this.without(this.snapshot.failures, key),
      saved: failure ? this.snapshot.saved : this.confirmed(key),
      completed: failure ? this.snapshot.completed : { ...this.snapshot.completed, [key]: (this.snapshot.completed[key] ?? 0) + 1 },
    });
    const text = this.domain.describe?.(action).text;
    if (text) this.announcer?.(failure ? `${text} could not be saved: ${failure}` : `${text}. Saved.`);
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

  /**
   * Marks the change pending, logs it and starts its timeout. Returns the number of this send, which
   * `fail` needs, or nothing when that item already has a change pending: then nothing is to be sent.
   */
  begin(action: A): number | undefined {
    const key = this.domain.keyOf(action);
    if (key in this.snapshot.pending) return undefined;
    this.failedActions.delete(key);
    this.publish({ pending: { ...this.snapshot.pending, [key]: action }, failures: this.without(this.snapshot.failures, key) });
    const describe = this.domain.describe?.(action);
    const logId = describe && this.activityApi?.add({ state: 'pending', lead: 'Waiting.', text: describe.text, subject: describe.subject });
    if (logId) this.logIds.set(key, logId);
    this.timers.set(key, setTimeout(() => this.settle(key, 'no confirmation from the server.'), this.domain.timeoutMs));
    const sendId = ++this.sendCount;
    this.sends.set(key, sendId);
    return sendId;
  }

  /** The request for send number `sendId` was refused or never arrived. An older send's answer is ignored. */
  fail(action: A, sendId: number, message: string) {
    const key = this.domain.keyOf(action);
    if (this.sends.get(key) === sendId) this.settle(key, message);
  }

  /**
   * Call whenever the cache holds new confirmed data. Settles each pending change the data decides,
   * and clears a failure once the data shows the value that failed after all. That late confirm does
   * not count as `completed`, so a form the person is editing is not rebuilt under them.
   */
  judge(data: D) {
    for (const [key, action] of Object.entries(this.snapshot.pending)) {
      const result = this.domain.verdict(action, data);
      if (result === 'confirmed') this.settle(key);
      else if (result !== 'waiting') this.settle(key, result.failed, true);
    }
    for (const [key, action] of this.failedActions) {
      if (this.domain.verdict(action, data) === 'confirmed') {
        this.failedActions.delete(key);
        this.publish({ failures: this.without(this.snapshot.failures, key), saved: this.confirmed(key) });
      }
    }
  }

  /** The query cache holds data of any shape. This store's query key names the entry it judges, so what arrives is `D`. */
  judgeCached(data: unknown) {
    this.judge(data as D);
  }

  dispose() {
    this.timers.forEach(clearTimeout);
    this.timers.clear();
  }
}
