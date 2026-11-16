import { useQuery } from '@tanstack/react-query';
import { CircleCheck, Link2, RefreshCw, UserPlus } from 'lucide-react';
import { ActivityTile, DrillView, Facts, List, ListRow, NeedsYouTile, PageHeader, Tile, TileGrid, useDrill, type Key, type NeedsYouItem } from '@heartwood/ui';
import { toEvents, useActivity } from '@/activity/activity-context';
import { states } from '@/brand';
import { lastSeen } from '@/clients/client-words';
import { ClientPage } from '@/clients/client-page';
import { clientName, useClients } from '@/clients/use-clients';
import { DevicePage } from '@/devices/device-page';
import { deviceWord, remoteEntries, remoteWord } from '@/devices/device-words';
import { RemoteDevicePage } from '@/devices/remote-device-page';
import { useClientDevices, useServerDevices } from '@/devices/use-devices';
import { usePending } from '@/pending/pending-context';
import { checksForRole } from '@/system/for-role';
import { pairingRequestsOptions, systemChecksOptions, systemInfoOptions } from '@/queries';
import { isClientRole, isServerRole, roleName } from '@/settings/roles';
import { useSettings } from '@/settings/use-settings';
import { useNow } from '@/shared/use-now';
import { goTo, useFrame } from '@/shell/frame-context';
import { Loading } from '@/shell/loading';

type Open = { kind: 'dev'; id: string } | { kind: 'rem'; id: string; serverId: string; busId: string } | { kind: 'cli'; id: string };

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
const stateOf = (word: string) => states[word] ?? { tone: 'neutral' };
const uptime = (startedAt: string, now: number) => {
  const hours = Math.max(0, Math.floor((now - Date.parse(startedAt)) / 3_600_000));
  return hours < 1 ? 'up under an hour' : hours < 48 ? `up ${plural(hours, 'hour', 'hours')}` : `up ${plural(Math.floor(hours / 24), 'day', 'days')}`;
};

/**
 * The machine at a glance: a sentence that says how it stands, Needs you when something does, then a
 * tile for each place that leads to its page. A row opens its device or client in this place, with a
 * way back to the overview.
 */
export function OverviewPage() {
  const { locked } = useFrame();
  const now = useNow();
  const settings = useSettings();
  const role = settings.query.data?.role;
  const server = isServerRole(role);
  const client = isClientRole(role);
  const serverDevices = useServerDevices(server).data;
  const clientDevices = useClientDevices(client).data;
  const clients = useClients(server).data;
  const info = useQuery(systemInfoOptions()).data;
  const checks = useQuery(systemChecksOptions()).data;
  const pairing = useQuery({ ...pairingRequestsOptions(), enabled: server }).data;
  const { items } = useActivity();
  const sharing = usePending('serverDevices');
  const attaching = usePending('clientDevices');
  const drill = useDrill<Open>();

  if (!settings.query.data) return <Loading />;
  const back = { label: 'Overview', onPress: drill.back };
  const top = drill.top;
  if (top) {
    return (
      <DrillView dir={drill.dir} viewKey={`${top.kind}-${top.id}`}>
        {top.kind === 'dev' ? <DevicePage busId={top.id} back={back} /> : top.kind === 'rem' ? <RemoteDevicePage serverId={top.serverId} busId={top.busId} back={back} /> : <ClientPage clientId={top.id} back={back} />}
      </DrillView>
    );
  }

  const host = info?.name ?? window.location.hostname;
  const devices = serverDevices?.devices ?? [];
  const sharedDevices = devices.filter((d) => d.state === 'shared' || d.state === 'attached');
  const attachedDevices = devices.filter((d) => d.state === 'attached');
  const servers = clientDevices?.servers ?? [];
  const remote = remoteEntries(servers);
  const here = remote.filter((r) => r.entry.attachedHere);
  const clientList = clients ?? [];
  const using = (id: string) => devices.filter((d) => d.state === 'attached' && d.attachedTo === id).map((d) => d.name);
  const usingClients = clientList.filter((c) => using(c.id).length);

  const needs: NeedsYouItem[] = [
    ...devices
      .filter((d) => d.fault)
      .map((d) => ({
        id: `fault-${d.busId}`,
        title: `${d.name} is not shared`,
        description: d.fault ?? undefined,
        action: { label: `Share ${d.name} again`, icon: RefreshCw, iconOnly: true, disabled: locked, pending: d.busId in sharing.pending, onPress: () => sharing.request({ busId: d.busId, share: true }) },
      })),
    ...remote
      .filter((r) => r.entry.fault)
      .map((r) => ({
        id: `attach-${r.key}`,
        title: `${r.entry.device.name} did not attach`,
        description: r.entry.fault ?? undefined,
        action: { label: `Attach ${r.entry.device.name} again`, icon: RefreshCw, iconOnly: true, disabled: locked, pending: r.key in attaching.pending, onPress: () => attaching.request({ serverId: r.server.id, busId: r.entry.device.busId, attach: true }) },
      })),
    ...servers
      .filter((s) => s.link === 'error')
      .map((s) => ({
        id: `link-${s.id}`,
        title: `Cannot reach ${s.name}`,
        description: 'Devices attached from it stop working until it is back.',
        action: { label: 'Open Connection', icon: Link2, onPress: () => goTo('connection') },
      })),
    ...checksForRole(checks ?? [], server, client)
      .filter((c) => !c.ok)
      .map((c) => ({ id: `check-${c.id}`, title: c.title, description: c.detail, action: { label: 'System check', icon: CircleCheck, onPress: () => goTo('check') } })),
    ...(pairing?.requests ?? [])
      .filter((r) => r.state === 'waiting')
      .map((r) => ({ id: `pair-${r.id}`, title: `${r.clientName} wants to pair`, description: `From ${r.from}. Type the code it shows on Clients.`, action: { label: 'Review', icon: UserPlus, onPress: () => goTo('clients') } })),
  ];

  const sentence =
    [
      server ? `${host} shares ${plural(sharedDevices.length, 'device', 'devices')}${attachedDevices.length ? `, ${attachedDevices.length} of them attached` : ''}` : host,
      client && servers.length ? `${server ? ', and' : ''} uses ${here.length} from ${servers.length === 1 ? servers[0].name : plural(servers.length, 'server', 'servers')}` : '',
    ].join('') + `. ${needs.length ? `${plural(needs.length, 'thing needs', 'things need')} you.` : 'Nothing needs you.'}`;

  const open = (item: Open) => drill.open(item);
  const rejected = items.filter((i) => i.state === 'fault').length;
  const remoteTitle = servers.length === 0 ? 'Remote devices' : servers.length === 1 ? `From ${servers[0].name}` : `From ${servers.length} servers`;

  // The grid has up to three columns. The device tiles share the first row so it is filled: one tile takes
  // the row, two give the devices the wider part, and three take a column each.
  const devicesSpan = server && client ? 1 : server ? 2 : ('full' as const);
  const clientsSpan = 1;

  return (
    <DrillView dir={drill.dir} viewKey="overview">
      <div className="kp-page">
        <PageHeader title={host} description={sentence} />
        <TileGrid label="Overview">
          <NeedsYouTile items={needs} />
          {server ? (
            <Tile title={`Sharing from ${host}`} span={devicesSpan} meta={`${sharedDevices.length} of ${devices.length}`} footerLink={{ label: 'All devices', onPress: () => goTo('devices') }}>
              {devices.length ? (
                <List label={`Devices on ${host}`} isDivided items={devices.slice(0, 5)} onAction={(id: Key) => open({ kind: 'dev', id: String(id) })}>
                  {(d) => {
                    const action = sharing.pending[d.busId];
                    const word = deviceWord(d);
                    return (
                      <ListRow
                        id={d.busId}
                        drill
                        status={action ? { tone: 'pending' } : stateOf(word)}
                        title={d.name}
                        description={d.state === 'attached' ? clientName(clientList, d.attachedTo) : (d.manufacturer ?? 'Unknown maker')}
                        statusLabel={action ? (action.share ? 'Sharing' : 'Stopping') : word}
                      />
                    );
                  }}
                </List>
              ) : (
                <p className="kp-muted kp-pad">Plug in a USB device to share it.</p>
              )}
            </Tile>
          ) : null}
          {client ? (
            <Tile title={remoteTitle} span={devicesSpan} meta={remote.length ? `${here.length} of ${remote.length}` : undefined} footerLink={{ label: 'All remote devices', onPress: () => goTo('remote') }}>
              {remote.length ? (
                <List label="Remote devices" isDivided items={remote.slice(0, 5)} onAction={(id: Key) => { const r = remote.find((x) => x.key === String(id)); if (r) open({ kind: 'rem', id: r.key, serverId: r.server.id, busId: r.entry.device.busId }); }}>
                  {(r) => {
                    const action = attaching.pending[r.key];
                    const word = remoteWord(r.entry);
                    return (
                      <ListRow
                        id={r.key}
                        drill
                        status={action ? { tone: 'pending' } : stateOf(word)}
                        title={r.entry.device.name}
                        description={r.server.name}
                        statusLabel={action ? (action.attach ? 'Attaching' : 'Detaching') : word}
                      />
                    );
                  }}
                </List>
              ) : (
                <p className="kp-muted kp-pad">
                  {servers.length === 0
                    ? 'Not paired with a server. Add one on Connection.'
                    : servers.some((s) => s.link === 'error')
                      ? 'Cannot reach the server. Koppel keeps trying.'
                      : 'The servers share no devices.'}
                </p>
              )}
            </Tile>
          ) : null}
          {server ? (
            <Tile title="Clients" span={clientsSpan} meta={`${usingClients.length} using devices`} footerLink={{ label: 'All clients', onPress: () => goTo('clients') }}>
              {clientList.length ? (
                <List label={`Clients of ${host}`} isDivided items={clientList.slice(0, 5)} onAction={(id: Key) => open({ kind: 'cli', id: String(id) })}>
                  {(c) => {
                    const names = using(c.id);
                    return (
                      <ListRow
                        id={c.id}
                        drill
                        status={names.length ? stateOf('Attached') : { tone: 'neutral' }}
                        title={c.name}
                        description={names.length ? names.join(', ') : `Last seen ${lastSeen(c, now).toLowerCase()}`}
                        statusLabel={names.length ? 'Attached' : 'Idle'}
                      />
                    );
                  }}
                </List>
              ) : (
                <p className="kp-muted kp-pad">No computer can attach yet.</p>
              )}
            </Tile>
          ) : null}
          <ActivityTile states={states} events={toEvents(items)} meta={`${rejected} rejected`} span={2} onOpen={() => goTo('activity')} />
          <Tile title="This computer" padded>
            <Facts
              label="This computer"
              items={[
                ...(server && info ? [{ label: 'Address', value: `${info.name}:${info.httpsPort}`, copy: true }, { label: 'Fingerprint', value: info.fingerprint, copy: true }] : []),
                { label: 'Role', value: roleName(role) },
                ...(info ? [{ label: 'Koppel', value: `${info.version}, ${uptime(info.startedAt, now)}` }] : []),
              ]}
            />
          </Tile>
        </TileGrid>
      </div>
    </DrillView>
  );
}
