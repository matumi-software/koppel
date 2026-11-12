import { useQuery } from '@tanstack/react-query';
import { Button, Caption, MiniList, Mono, Panel, PanelGrid, StatusBadge, type MiniListItem } from '@koppel/react-ui';
import { useActivity } from '@/activity/activity-context';
import { hostOf, since, useClientDevices, useServerDevices } from '@/devices/use-devices';
import { isClientRole, isServerRole } from '@/settings/roles';
import { Frame } from '@/shell/frame';
import { useMachineName } from '@/shared/use-machine-name';
import { goTo } from '@/shell/frame-context';
import { clientsOptions, connectionOptions, settingsOptions } from '@/queries';

/** Everything at a glance: what needs you, what this machine shares, and what it has attached. */
export function OverviewPage() {
  const host = useMachineName();
  const { items } = useActivity();
  const settings = useQuery(settingsOptions());
  const isServer = isServerRole(settings.data?.role);
  const isClient = isClientRole(settings.data?.role);
  const server = useServerDevices(isServer);
  const client = useClientDevices(isClient);
  const connection = useQuery({ ...connectionOptions(), enabled: isClient });
  const remote = hostOf(connection.data?.serverUrl) ?? 'the server';
  const clients = useQuery({ ...clientsOptions(), enabled: isServer });
  const clientName = (id: string | null) => clients.data?.find((c) => c.id === id)?.name ?? id ?? 'A client';

  const needs: MiniListItem[] = [];
  for (const item of items.filter((i) => i.state === 'pending')) {
    needs.push({ id: item.id, name: item.text, end: <StatusBadge state="pending" />, sub: <Caption>Waiting for the server to confirm.</Caption> });
  }
  for (const d of isServer ? (server.data?.devices ?? []) : []) {
    if (d.fault) {
      needs.push({ id: `s-${d.busId}`, name: `${d.name} was not changed`, end: <Button variant="secondary" onPress={() => goTo('devices')}>Open</Button>, sub: <Caption tone="fault">{d.fault}</Caption> });
    }
  }
  for (const e of isClient ? (client.data?.devices ?? []) : []) {
    if (e.fault) {
      needs.push({ id: `c-${e.device.busId}`, name: `${e.device.name} on ${remote}`, end: <Button variant="secondary" onPress={() => goTo('attach')}>Open</Button>, sub: <Caption tone="fault">{e.fault}</Caption> });
    }
  }
  if (isServer && server.data?.unavailable) {
    needs.push({ id: 'share-unavailable', name: 'This machine cannot share devices', sub: <Caption tone="fault">{server.data.unavailable}</Caption> });
  }
  if (isClient && client.data?.link === 'unconfigured') {
    needs.push({ id: 'no-connection', name: 'No server connection yet', end: <Button variant="secondary" onPress={() => goTo('configuration')}>Set up</Button>, sub: 'Add the server and the client ID and secret it gave you.' });
  }
  if (isClient && client.data?.link === 'error') {
    needs.push({ id: 'link-error', name: `Cannot reach ${remote}`, sub: <Caption tone="fault">{client.data.linkMessage}</Caption> });
  }

  const shared = (server.data?.devices ?? []).filter((d) => d.state !== 'local');
  const sharing: MiniListItem[] = shared.map((d) => ({
    id: d.busId,
    name: d.name,
    end: <StatusBadge state={d.state} />,
    sub: (
      <>
        <Mono>{d.vidPid || d.busId}</Mono>
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
        <Mono>{e.device.vidPid}</Mono>
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
      <PanelGrid>
        {isServer && (
          <Panel title={`Sharing from ${host}`} meta={`${shared.length} of ${server.data?.devices.length ?? 0}`} footer={<Button variant="ghost" onPress={() => goTo('devices')}>All devices</Button>}>
            <MiniList items={sharing} empty="Nothing shared yet. Open Devices and switch Share on." />
          </Panel>
        )}
        {isClient && (
          <Panel title={`Attached from ${remote}`} footer={<Button variant="ghost" onPress={() => goTo('attach')}>All devices on {remote}</Button>}>
            <MiniList items={attached} empty={client.data?.link === 'connected' ? `${remote} is not sharing any devices.` : 'Not connected to a server.'} />
          </Panel>
        )}
      </PanelGrid>
    </Frame>
  );
}
