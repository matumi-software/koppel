import type { QueryClient, QueryKey } from '@tanstack/react-query';
import { mergeActivity } from '@/activity/activity-context';
import type { ActivityEvent } from '@/api/activity';
import { activityOptions, clientDevicesOptions, clientsOptions, connectionOptions, discoveredServersOptions, pairingRequestsOptions, pairingStatusOptions, serverDevicesOptions, serverKeysOptions, settingsOptions, systemInfoOptions } from '@/queries';

const activityKey = activityOptions().queryKey;

/** Where the server pushes, as Server-Sent Events. */
export const eventsUrl = '/api/events';

interface Push<T = unknown> {
  /** The cache entries this event keeps current: refetched when a payload cannot be applied. */
  keys: readonly QueryKey[];
  // A method, not a property, so a push of one wire type fits the table's `Push<unknown>`.
  apply(queryClient: QueryClient, payload: T): void;
}

/**
 * Applies a push that carries the whole value. A fetch still in flight would land after it with older
 * data, so it is cancelled first. The cancel reverts the query to the state before that fetch, and does
 * it synchronously, so setting the data straight after it cannot be undone and two pushes for one key
 * land in the order they came.
 */
function replaceWith(queryClient: QueryClient, key: QueryKey, value: unknown) {
  void queryClient.cancelQueries({ queryKey: key });
  queryClient.setQueryData(key, value);
}

const whole = (key: QueryKey, alsoStale: readonly QueryKey[] = []): Push => ({
  keys: [key, ...alsoStale],
  apply: (queryClient, payload) => {
    replaceWith(queryClient, key, payload);
    // The side panel shows what this event changes (role, USB port, fingerprint): it is fetched, not pushed.
    for (const stale of alsoStale) void queryClient.invalidateQueries({ queryKey: stale });
  },
});

/**
 * Every event the server pushes and what it does to the query cache, whichever page is open. Pages
 * read the cache and judge only their own pending changes (see PendingProvider). The names to listen
 * for come from here too.
 */
export const pushTable: Record<string, Push> = {
  SettingsChanged: whole(settingsOptions().queryKey, [systemInfoOptions().queryKey]),
  ClientsChanged: whole(clientsOptions().queryKey),
  ServerKeysChanged: whole(serverKeysOptions().queryKey, [systemInfoOptions().queryKey]),
  ConnectionChanged: whole(connectionOptions().queryKey),
  DevicesChanged: whole(serverDevicesOptions().queryKey),
  ClientDevicesChanged: whole(clientDevicesOptions().queryKey),
  PairingRequestsChanged: whole(pairingRequestsOptions().queryKey),
  PairingStatusChanged: whole(pairingStatusOptions().queryKey),
  ServersDiscovered: whole(discoveredServersOptions().queryKey),
  // One event, not the whole list: merge it in by id, and if the list never loaded, load it.
  // The fetch in flight is not cancelled: it merges with the cache after it lands, so it keeps this event.
  ActivityRecorded: {
    keys: [activityKey],
    apply: (queryClient, event: ActivityEvent) => {
      const loaded = queryClient.getQueryData(activityKey) !== undefined;
      queryClient.setQueryData<ActivityEvent[]>(activityKey, (current) => mergeActivity(current, [event]));
      if (!loaded) void queryClient.invalidateQueries({ queryKey: activityKey });
    },
  },
};

export const pushEvents = Object.keys(pushTable);

/**
 * Applies one push event. A payload that is not JSON, or that the cache entry cannot take, is logged
 * and the entry it belongs to is fetched instead, so the cache never stays stale without a trace.
 */
export function applyPush(queryClient: QueryClient, name: string, data: string) {
  const push = pushTable[name];
  if (!push) return;
  try {
    push.apply(queryClient, JSON.parse(data));
  } catch (error) {
    console.error(`Koppel: could not apply the ${name} push, fetching its data instead.`, error);
    for (const key of push.keys) void queryClient.invalidateQueries({ queryKey: key });
  }
}

const retryDelays = [0, 2000, 5000, 10000, 30000];

/** How long to wait before reconnect number `attempt` (from 0). It keeps retrying; the last delay repeats. */
export const retryDelay = (attempt: number) => retryDelays[Math.min(attempt, retryDelays.length - 1)];

/** Failures after which the link has been away long enough, at the last step, to call it down. */
export const downAfterFailures = retryDelays.length - 1;
