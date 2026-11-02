import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { ActivityFeed, Facts } from '@koppel/react-ui';
import { toEntries, useActivity } from '@/activity/activity-context';
import { getSystemInfo } from '@/api/http';
import { useHubEvent } from '@/hub/hub-context';

const uptime = (since: string) => {
  const minutes = Math.max(0, Math.round((Date.now() - new Date(since).getTime()) / 60_000));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h ${minutes % 60} min`;
  return `${Math.floor(hours / 24)} days`;
};

const roles = { server: 'Server', client: 'Client', both: 'Server and client' } as const;
const infoKey = ['system-info'] as const;

/**
 * The side panel when nothing is selected: facts about this machine, then the activity log, which
 * the server keeps, so it is the same after a reload and on every device.
 */
export function SideInfo() {
  const { items } = useActivity();
  const queryClient = useQueryClient();
  const info = useQuery({ queryKey: infoKey, queryFn: getSystemInfo, refetchInterval: 60_000 });

  // The role and USB port come from the settings, so a confirmed settings change refetches them.
  const onSettingsChanged = useCallback(() => void queryClient.invalidateQueries({ queryKey: infoKey }), [queryClient]);
  useHubEvent('SettingsChanged', onSettingsChanged);
  const data = info.data;

  return (
    <>
      {data && (
        <Facts
          title={data.name}
          facts={[
            { label: 'Role', value: roles[data.role] },
            { label: 'Up for', value: uptime(data.startedAt) },
            { label: 'Version', value: data.version, mono: true },
            { label: 'Ports', value: `UI ${data.uiPort} · API ${data.httpsPort} · USB ${data.usbPort}`, mono: true },
            { label: 'Fingerprint', value: `${data.fingerprint.slice(0, 23)}…`, mono: true },
            { label: 'System', value: data.os },
          ]}
        />
      )}
      <h2 className="kp-type-heading" style={{ margin: 0 }}>
        Activity
      </h2>
      <ActivityFeed entries={toEntries(items)} empty="Changes show here when the server confirms or rejects them." />
    </>
  );
}
