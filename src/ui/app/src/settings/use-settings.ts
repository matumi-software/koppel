import { useQuery } from '@tanstack/react-query';
import { useCallback } from 'react';
import type { KoppelRole, KoppelSettings } from '@/api/settings';
import { setting, setupKey } from '@/pending/domains';
import { usePending } from '@/pending/pending-context';
import { settingsOptions } from '@/queries';

type Key = keyof KoppelSettings;

export interface SettingStatus<K extends Key> {
  /** The value the server last confirmed. */
  confirmed: KoppelSettings[K];
  /** The value the user asked for, until the server confirms it. */
  pendingTarget?: KoppelSettings[K];
  failure?: string;
  /** True after the server confirmed a change the user made in this session. */
  saved: boolean;
}

/**
 * Settings with the confirm rule: a change is pending from the moment it is sent until the settings
 * the server pushes (or a refetch) show the same value, after the server wrote its file. The pending
 * state lives above the page, so it survives leaving it.
 */
export function useSettings() {
  const query = useQuery(settingsOptions());
  const { pending, failures, saved, request } = usePending('settings');

  const requestChange = useCallback(
    <K extends Key>(key: K, value: KoppelSettings[K]) => request(setting(key, value)),
    [request],
  );

  /** First run: the role and "setup complete" go in one PATCH, so they are saved or not saved together. */
  const requestSetup = useCallback(
    (role: KoppelRole) => request({ type: 'setup', role }),
    [request],
  );

  const pendingValue = <K extends Key>(key: K) => {
    const action = pending[key];
    return action?.type === 'set' ? (action.value as KoppelSettings[K]) : undefined;
  };

  const status = <K extends Key>(key: K): SettingStatus<K> | undefined =>
    query.data && {
      confirmed: query.data[key],
      pendingTarget: pendingValue(key),
      failure: failures[key],
      saved: (saved[key] ?? 0) > 0,
    };

  const setup = { isPending: setupKey in pending, failure: failures[setupKey] };

  return { query, status, requestChange, requestSetup, setup };
}
