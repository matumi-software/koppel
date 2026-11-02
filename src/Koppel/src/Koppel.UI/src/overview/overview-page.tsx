import { useQuery } from '@tanstack/react-query';
import { Button, MiniList, Panel, StatusBadge, type MiniListItem } from '@koppel/react-ui';
import { useActivity } from '@/activity/activity-context';
import { getConnection } from '@/api/client';
import { getClients } from '@/api/server';
import { getSettings } from '@/api/settings';
import { hostOf, since, useClientDevices, useServerDevices } from '@/devices/use-devices';
import { Frame } from '@/shell/frame';
import { useMachineName } from '@/shared/use-machine-name';
import { useFrame } from '@/shell/frame-context';

/** Everything at a glance: what needs you, what this machine shares, and what it has attached. */
export function OverviewPage() {
  const host = useMachineName();
  const { go } = useFrame();
  const { items } = useActivity();
  const settings = useQuery({ queryKey: ['settings'], queryFn: getSettings });
  const role = settings.data?.role ?? 'server';
  const isServer = role !== 'client';
  const isClient = role !== 'server';
  const server = useServerDevices(isServer);
  const client = useClientDevices(isClient);
  const connection = useQuery({ queryKey: ['connection'], queryFn: getConnection, enabled: isClient });
  const remote = hostOf(connection.data?.serverUrl) ?? 'the server';
  const clients = useQuery({ queryKey: ['clients'], queryFn: getClients, enabled: isServer });
  const clientName = (id: string | null) => clients.data?.find((c) => c.id === id)?.name ?? id ?? 'A client';

  const needs: MiniListItem[] = [];
  for (const item of items.filter((i) => i.state === 'pending')) {
    needs.push({ id: item.id, name: item.text, end: <StatusBadge state="pending" />, sub: <span className="kp-type-caption">Waiting for the server to confirm.</span> });
  }
  for (const d of isServer ? (server.data?.devices ?? []) : []) {
    if (d.fault) {
      needs.push({ id: `s-${d.busId}`, name: `${d.name} was not changed`, end: <Button variant="secondary" onPress={() => go('devices')}>Open</Button>, sub: <span style={{ color: 'var(--fault-text)' }}>{d.fault}</span> });
    }
  }
  for (const e of isClient ? (client.data?.devices ?? []) : []) {
    if (e.fault) {
      needs.push({ id: `c-${e.device.busId}`, name: `${e.device.name} on ${remote}`, end: <Button variant="secondary" onPress={() => go('attach')}>Open</Button>, sub: <span style={{ color: 'var(--fault-text)' }}>{e.fault}</span> });
    }
  }
  if (isServer && server.data?.unavailable) {
    needs.push({ id: 'share-unavailable', name: 'This machine cannot share devices', sub: <span style={{ color: 'var(--fault-text)' }}>{server.data.unavailable}</span> });
  }
  if (isClient && client.data?.link === 'unconfigured') {
    needs.push({ id: 'no-connection', name: 'No server connection yet', end: <Button variant="secondary" onPress={() => go('configuration')}>Set up</Button>, sub: 'Add the server and the client ID and secret it gave you.' });
  }
  if (isClient && client.data?.link === 'error') {
    needs.push({ id: 'link-error', name: `Cannot reach ${remote}`, sub: <span style={{ color: 'var(--fault-text)' }}>{client.data.linkMessage}</span> });
  }

  const shared = (server.data?.devices ?? []).filter((d) => d.state !== 'local');
  const sharing: MiniListItem[] = shared.map((d) => ({
    id: d.busId,
    name: d.name,
    end: <StatusBadge state={d.state} />,
    sub: (
      <>
        <span className="kp-type-mono-sm">{d.vidPid || d.busId}</span>
        <span>{d.state === 'attached' ? `${clientName(d.attachedTo)}${since(d.attachedSince)}` : d.state === 'offline' ? 'Unplugged' : 'Free'}</span>
      </>
    ),
  }));
  const attached: MiniListItem[] = (client.data?.devices ?? []).map((e) => ({
    id: e.device.busId,
    name: e.device.name,
    end: <StatusBadge state={e.attachedHere ? 'attached' : e.device.state} detail={e.attachedHere ? 'here' : undefined} />,
    sub: (
      <>
        <span className="kp-type-mono-sm">{e.device.vidPid}</span>
        <span>{e.attachedHere ? `Virtual port ${e.port}${since(e.attachedSince)}` : e.device.state === 'attached' ? 'In use elsewhere' : 'Free to attach'}</span>
      </>
    ),
  }));

  const attachedHere = (client.data?.devices ?? []).filter((e) => e.attachedHere).length;
  const lede = [
    isServer && `${host} shares ${shared.length} ${shared.length === 1 ? 'device' : 'devices'}`,
    isClient && `${isServer ? 'has' : `${host} has`} ${attachedHere === 0 ? 'nothing' : attachedHere} attached from ${remote}`,
  ]
    .filter(Boolean)
    .join(' and ');

  return (
    <Frame
      title="Overview"
      lede={`${lede}.${needs.length ? ` ${needs.length} ${needs.length === 1 ? 'thing needs' : 'things need'} you.` : ' Nothing needs you.'}`}
    >
      {needs.length > 0 && (
        <Panel title="Needs you" meta={needs.length} tone="needs">
          <MiniList items={needs} empty="" />
        </Panel>
      )}
      <div className="kp-panels">
        {isServer && (
          <Panel title={`Sharing from ${host}`} meta={`${shared.length} of ${server.data?.devices.length ?? 0}`} footer={<Button variant="ghost" onPress={() => go('devices')}>All devices</Button>}>
            <MiniList items={sharing} empty="Nothing shared yet. Open Devices and switch Share on." />
          </Panel>
        )}
        {isClient && (
          <Panel title={`Attached from ${remote}`} footer={<Button variant="ghost" onPress={() => go('attach')}>All devices on {remote}</Button>}>
            <MiniList items={attached} empty={client.data?.link === 'connected' ? `${remote} is not sharing any devices.` : 'Not connected to a server.'} />
          </Panel>
        )}
      </div>
    </Frame>
  );
}
