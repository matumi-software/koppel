import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useRef, useState } from 'react';
import { getSettings, patchSettings, type KoppelSettings } from '@/api/settings';
import { useHubEvent } from '@/hub/hub-context';

type Key = keyof KoppelSettings;

/** How long to wait for the server's confirmation before calling a change failed. */
const confirmTimeoutMs = 10_000;

export interface SettingStatus<K extends Key> {
  /** The value the server last confirmed. */
  confirmed: KoppelSettings[K];
  /** The value the user asked for, until the server confirms it. */
  pendingTarget?: KoppelSettings[K];
  failure?: string;
  /** True after the server confirmed a change the user made in this session. */
  saved: boolean;
}

const settingsKey = ['settings'] as const;

/**
 * Settings with the confirm-on-push rule: a change is pending from the moment it is sent until
 * SettingsChanged brings the same value back from the server, after the server wrote its file.
 */
export function useSettings() {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: settingsKey, queryFn: getSettings });
  const [pending, setPending] = useState<Partial<Record<Key, unknown>>>({});
  const [failures, setFailures] = useState<Partial<Record<Key, string>>>({});
  const [saved, setSaved] = useState<Partial<Record<Key, boolean>>>({});
  const timers = useRef(new Map<Key, ReturnType<typeof setTimeout>>());
  // The pending map is written through this ref first, so hub handlers and the request callback
  // always see the latest value without re-subscribing.
  const pendingRef = useRef(pending);

  const settle = useCallback((key: Key, failure?: string) => {
    clearTimeout(timers.current.get(key));
    timers.current.delete(key);
    const rest = { ...pendingRef.current };
    delete rest[key];
    pendingRef.current = rest;
    setPending(rest);
    setFailures((current) => ({ ...current, [key]: failure }));
    setSaved((current) => ({ ...current, [key]: failure === undefined }));
  }, []);

  const onSettingsChanged = useCallback(
    (settings: KoppelSettings) => {
      queryClient.setQueryData(settingsKey, settings);
      const current = pendingRef.current;
      for (const key of Object.keys(current) as Key[]) {
        if (settings[key] === current[key]) {
          settle(key);
        }
      }
    },
    [queryClient, settle],
  );
  useHubEvent('SettingsChanged', onSettingsChanged);

  useEffect(() => {
    const active = timers.current;
    return () => active.forEach(clearTimeout);
  }, []);

  const requestChange = useCallback(
    <K extends Key>(key: K, value: KoppelSettings[K]) => {
      if (key in pendingRef.current) return;
      pendingRef.current = { ...pendingRef.current, [key]: value };
      setPending(pendingRef.current);
      setFailures((current) => ({ ...current, [key]: undefined }));
      setSaved((current) => ({ ...current, [key]: false }));
      timers.current.set(
        key,
        setTimeout(() => settle(key, 'no confirmation from the server.'), confirmTimeoutMs),
      );
      patchSettings({ [key]: value }).catch((error: unknown) =>
        settle(key, error instanceof Error ? error.message : 'the request failed.'),
      );
    },
    [settle],
  );

  const status = <K extends Key>(key: K): SettingStatus<K> | undefined =>
    query.data && {
      confirmed: query.data[key],
      pendingTarget: key in pending ? (pending[key] as KoppelSettings[K]) : undefined,
      failure: failures[key],
      saved: saved[key] ?? false,
    };

  return { query, status, requestChange };
}
