import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { getSettings, patchSettings, type KoppelSettings } from '@/api/settings';
import { useHubEvent } from '@/hub/hub-context';
import { usePending } from '@/shared/use-pending';

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

const settingsKey = ['settings'] as const;

const labels: Record<Key, string> = {
  role: 'Role',
  shareOnStartup: 'Share saved devices on startup',
  listenPort: 'Listen port',
  setupComplete: 'Setup',
};

const describeSetting = (key: string, value: unknown) => ({
  text:
    key === 'setupComplete'
      ? 'Finish setup'
      : `${labels[key as Key] ?? key}: ${typeof value === 'boolean' ? (value ? 'on' : 'off') : String(value)}`,
});

/**
 * Settings with the confirm rule: a change is pending from the moment it is sent until
 * SettingsChanged brings the same value back from the server, after the server wrote its file.
 */
export function useSettings() {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: settingsKey, queryFn: getSettings });
  const { pending, failures, saved, request, judge } = usePending<unknown>({ describe: describeSetting });

  const onSettingsChanged = useCallback(
    (settings: KoppelSettings) => {
      queryClient.setQueryData(settingsKey, settings);
      judge((key, target) => (settings[key as Key] === target ? 'confirmed' : 'waiting'));
    },
    [queryClient, judge],
  );
  useHubEvent('SettingsChanged', onSettingsChanged);

  const requestChange = useCallback(
    <K extends Key>(key: K, value: KoppelSettings[K]) => request(key, value, () => patchSettings({ [key]: value })),
    [request],
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
