import { useQuery } from '@tanstack/react-query';
import { ActivityFeed, Facts } from '@koppel/react-ui';
import { toEntries, useActivity } from '@/activity/activity-context';
import { systemInfoOptions } from '@/queries';
import { useNow } from './use-now';

const uptime = (since: string, now: number) => {
  const minutes = Math.max(0, Math.round((now - new Date(since).getTime()) / 60_000));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h ${minutes % 60} min`;
  return `${Math.floor(hours / 24)} days`;
};

const roles: Record<string, string> = { server: 'Server', client: 'Client', both: 'Server and client' };

/**
 * The side panel when nothing is selected: facts about this machine, then the activity log, which
 * the server keeps, so it is the same after a reload and on every device.
 */
export function SideInfo() {
  const { items } = useActivity();
  const now = useNow();
  // The role and USB port come from the settings, so the live push refetches this on SettingsChanged.
  const info = useQuery(systemInfoOptions());
  const data = info.data;

  return (
    <>
      {data && (
        <Facts
          title={data.name}
          facts={[
            { label: 'Role', value: roles[data.role] ?? 'Unknown' },
            { label: 'Up for', value: uptime(data.startedAt, now) },
            { label: 'Version', value: data.version, mono: true },
            { label: 'Ports', value: data.uiPort === data.httpsPort ? `${data.uiPort}` : `UI ${data.uiPort} · Network ${data.httpsPort}`, mono: true },
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
