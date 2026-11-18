import { useState } from 'react';
import { CircleAlert, Plug, RefreshCw } from 'lucide-react';
import { Button, Callout, DrillView, EmptyState, PageHeader, SearchField, StateLabel, StatusDot, useDrill } from '@heartwood/ui';
import { states } from '@/brand';
import { usePending } from '@/pending/pending-context';
import { goTo } from '@/shell/frame-context';
import { QueryGate } from '@/shell/loading';
import { CardGroup, DeviceCard } from './device-card';
import { AttachSwitch } from './device-switches';
import { remoteEntries, remoteSub, remoteWord, serversTitle } from './device-words';
import { RemoteDevicePage } from './remote-device-page';
import { useClientDevices } from './use-devices';

const groups = [
  { title: 'Available', words: ['Not attached', 'Attached'] },
  { title: 'Not available', words: ['In use', 'Offline'] },
];

/**
 * The devices of every server this machine attaches from, together, each saying which server it is on.
 * Attaching is a switch on each tile, pending until ClientDevicesChanged shows it attached here.
 */
export function RemoteDevicesPage() {
  const query = useClientDevices();
  const { pending, failures } = usePending('clientDevices');
  const drill = useDrill<{ serverId: string; busId: string }>();
  const [q, setQ] = useState('');

  if (!query.data) return <QueryGate query={query} />;
  const { servers, unavailable, gatewayFault } = query.data;
  const entries = remoteEntries(servers);
  const open = drill.top;
  if (open) {
    return (
      <DrillView dir={drill.dir} viewKey={`remote-${open.serverId}-${open.busId}`}>
        <RemoteDevicePage serverId={open.serverId} busId={open.busId} back={{ label: 'Remote devices', onPress: drill.back }} />
      </DrillView>
    );
  }

  if (servers.length === 0) {
    return (
      <div className="kp-fill">
        <EmptyState
          level={2}
          icon={Plug}
          title="Add a server to attach its devices"
          actions={
            <Button variant="primary" onPress={() => goTo('connection')}>
              Add a server
            </Button>
          }
        >
          Show a pairing code under Clients on the server, the computer running Koppel that shares the devices.
        </EmptyState>
      </div>
    );
  }

  const needle = q.trim().toLowerCase();
  const shown = entries.filter((r) => !needle || [r.entry.device.name, r.entry.device.manufacturer, r.entry.device.vidPid, r.server.name].join(' ').toLowerCase().includes(needle));
  const away = servers.filter((s) => s.link === 'error' || s.link === 'connecting');
  return (
    <DrillView dir={drill.dir} viewKey="remote">
      <div className="kp-page">
        <PageHeader
          title={serversTitle(servers)}
          actions={entries.length ? <SearchField aria-label="Search devices" placeholder="Search devices" shortcut="Ctrl K" size="md" value={q} onChange={setQ} width="var(--size-width-xs)" /> : undefined}
        />
        {unavailable ? (
          <Callout tone="danger" title="Devices cannot be attached">
            {unavailable}
          </Callout>
        ) : null}
        {gatewayFault && gatewayFault !== unavailable ? (
          <Callout tone="warning" title="The USB/IP gateway is not listening">
            {gatewayFault}
          </Callout>
        ) : null}
        {away.map((s) => (
          <Callout
            key={s.id}
            tone={s.link === 'error' ? 'danger' : 'info'}
            icon={s.link === 'error' ? CircleAlert : RefreshCw}
            title={s.link === 'error' ? `Cannot reach ${s.name}` : `Connecting to ${s.name}`}
            actions={
              s.link === 'error' ? (
                <Button variant="secondary" onPress={() => goTo('connection')}>
                  Open Connection
                </Button>
              ) : undefined
            }
          >
            {s.link === 'error' ? `${s.linkMessage ?? 'Check that it is on and on this network.'} Koppel keeps trying.` : 'Its devices show once the link is up.'}
          </Callout>
        ))}
        {entries.length > 0 && shown.length === 0 ? (
          <EmptyState level={2} title={`No devices match "${q}"`} />
        ) : (
          groups.map((group) => {
            const items = shown.filter((r) => group.words.includes(remoteWord(r.entry, r.server.link)));
            if (!items.length) return null;
            return (
              <CardGroup key={group.title} title={group.title} count={items.length}>
                {items.map((r) => {
                  const action = pending[r.key];
                  const fault = failures[r.key] ?? r.entry.fault;
                  const word = remoteWord(r.entry, r.server.link);
                  return (
                    <DeviceCard
                      key={r.key}
                      id={r.key}
                      name={r.entry.device.name}
                      sub={remoteSub(r.entry, r.server.name, fault, r.server.link)}
                      subDanger={!!fault}
                      state={action ? <StatusDot tone="pending">{action.attach ? 'Attaching' : 'Detaching'}</StatusDot> : <StateLabel states={states} state={word} />}
                      control={<AttachSwitch item={r} bare />}
                      onOpen={() => drill.open({ serverId: r.server.id, busId: r.entry.device.busId })}
                    />
                  );
                })}
              </CardGroup>
            );
          })
        )}
      </div>
    </DrillView>
  );
}
