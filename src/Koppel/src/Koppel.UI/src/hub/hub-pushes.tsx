import { useQueryClient, type QueryClient, type QueryKey } from '@tanstack/react-query';
import { useEffect } from 'react';
import { mergeActivity } from '@/activity/activity-context';
import type { ServerActivity } from '@/api/activity';
import type { KoppelSettings } from '@/api/settings';
import {
  activityKey,
  clientDevicesKey,
  clientsKey,
  connectionKey,
  discoveredServersKey,
  pairingRequestsKey,
  pairingStatusKey,
  serverDevicesKey,
  settingsKey,
  systemInfoKey,
} from '@/query-keys';
import { useHub } from './hub-context';

/**
 * Applies a push that carries the whole value to the cache. A fetch still in flight would land after
 * it with older data, so it is cancelled first.
 */
async function apply<T>(queryClient: QueryClient, key: QueryKey, next: T | ((current: T | undefined) => T)) {
  await queryClient.cancelQueries({ queryKey: key });
  queryClient.setQueryData<T>(key, next as T);
}

/** A push that carries the whole value. */
const replace = (queryClient: QueryClient, key: QueryKey) => (value: unknown) => void apply(queryClient, key, value);

/**
 * The one place every hub push reaches the query cache, whichever page is open. Pages read the cache
 * and judge only their own pending changes (see PendingProvider). Nothing else may register for these
 * events, or a push would be applied twice.
 */
export function HubPushes() {
  const { connection } = useHub();
  const queryClient = useQueryClient();

  useEffect(() => {
    const handlers: Record<string, (payload: unknown) => void> = {
      SettingsChanged: (settings) => {
        void apply<KoppelSettings>(queryClient, settingsKey, settings as KoppelSettings);
        // The role and USB port in the side panel come from the settings.
        void queryClient.invalidateQueries({ queryKey: systemInfoKey });
      },
      ClientsChanged: replace(queryClient, clientsKey),
      ConnectionChanged: replace(queryClient, connectionKey),
      DevicesChanged: replace(queryClient, serverDevicesKey),
      ClientDevicesChanged: replace(queryClient, clientDevicesKey),
      PairingRequestsChanged: replace(queryClient, pairingRequestsKey),
      PairingStatusChanged: replace(queryClient, pairingStatusKey),
      ServersDiscovered: replace(queryClient, discoveredServersKey),
      // One event, not the whole list: merge it in by id, and if the list never loaded, load it.
      // The fetch in flight is not cancelled: it merges with the cache after it lands, so it keeps this event.
      ActivityRecorded: (event) => {
        const loaded = queryClient.getQueryData(activityKey) !== undefined;
        queryClient.setQueryData<ServerActivity[]>(activityKey, (current) => mergeActivity(current, [event as ServerActivity]));
        if (!loaded) void queryClient.invalidateQueries({ queryKey: activityKey });
      },
    };
    for (const [name, handler] of Object.entries(handlers)) connection.on(name, handler);
    return () => {
      for (const [name, handler] of Object.entries(handlers)) connection.off(name, handler);
    };
  }, [connection, queryClient]);

  return null;
}
