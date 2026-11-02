import { useCallback, useEffect, useRef, useState } from 'react';

/** How long to wait for the server's confirmation before calling a change failed. */
export const confirmTimeoutMs = 10_000;

/** What a push says about one pending change. */
export type Verdict = 'confirmed' | 'waiting' | { failed: string };

/**
 * Pending changes keyed by item, for the confirm rule: a change is pending from the moment it is
 * sent until a push confirms it, fails it, or the timeout runs out.
 */
export function usePending<T>(timeoutMs = confirmTimeoutMs) {
  const [pending, setPending] = useState<Readonly<Record<string, T>>>({});
  const [failures, setFailures] = useState<Readonly<Record<string, string>>>({});
  // Written before state so push handlers always see the latest map.
  const pendingRef = useRef<Record<string, T>>({});
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  const settle = useCallback((key: string, failure?: string) => {
    clearTimeout(timers.current.get(key));
    timers.current.delete(key);
    const rest = { ...pendingRef.current };
    delete rest[key];
    pendingRef.current = rest;
    setPending(rest);
    setFailures((current) => {
      const next = { ...current };
      if (failure) next[key] = failure;
      else delete next[key];
      return next;
    });
  }, []);

  /** Marks the change pending and sends it. A refused request fails it at once. */
  const request = useCallback(
    (key: string, target: T, send: () => Promise<void>) => {
      if (key in pendingRef.current) return;
      pendingRef.current = { ...pendingRef.current, [key]: target };
      setPending(pendingRef.current);
      setFailures((current) => {
        const next = { ...current };
        delete next[key];
        return next;
      });
      timers.current.set(key, setTimeout(() => settle(key, 'no confirmation from the server.'), timeoutMs));
      send().catch((error: unknown) => settle(key, error instanceof Error ? error.message : 'the request failed.'));
    },
    [settle, timeoutMs],
  );

  /** Call on every push: asks what it means for each pending change. */
  const judge = useCallback(
    (verdict: (key: string, target: T) => Verdict) => {
      for (const [key, target] of Object.entries(pendingRef.current)) {
        const result = verdict(key, target);
        if (result === 'confirmed') settle(key);
        else if (result !== 'waiting') settle(key, result.failed);
      }
    },
    [settle],
  );

  useEffect(() => {
    const active = timers.current;
    return () => active.forEach(clearTimeout);
  }, []);

  return { pending, failures, request, judge };
}
