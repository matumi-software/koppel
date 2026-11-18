import { useState } from 'react';
import { Usb } from 'lucide-react';
import { Callout, DrillView, EmptyState, PageHeader, SearchField, StateLabel, StatusDot, useDrill } from '@heartwood/ui';
import { states } from '@/brand';
import { clientName, useClients } from '@/clients/use-clients';
import { usePending } from '@/pending/pending-context';
import { useMachineName } from '@/shared/use-machine-name';
import { QueryGate } from '@/shell/loading';
import { CardGroup, DeviceCard } from './device-card';
import { DevicePage } from './device-page';
import { ShareSwitch } from './device-switches';
import { deviceSub, deviceWord } from './device-words';
import { useServerDevices } from './use-devices';

const groups = [
  { title: 'Available', states: ['attached', 'shared', 'local'] },
  { title: 'Not available', states: ['offline'] },
];

/**
 * The devices on this machine as tiles grouped by what the person cannot change (plugged in or not).
 * Sharing is a switch on each tile, pending until DevicesChanged confirms it; a tile opens the device's page.
 */
export function DevicesPage() {
  const host = useMachineName();
  const query = useServerDevices();
  const clients = useClients();
  const { pending, failures } = usePending('serverDevices');
  const drill = useDrill<string>();
  const [q, setQ] = useState('');

  if (!query.data) return <QueryGate query={query} />;
  const { devices, unavailable } = query.data;
  const open = drill.top;
  if (open) {
    return (
      <DrillView dir={drill.dir} viewKey={`device-${open}`}>
        <DevicePage busId={open} back={{ label: 'Devices', onPress: drill.back }} />
      </DrillView>
    );
  }

  const needle = q.trim().toLowerCase();
  const shown = devices.filter((d) => !needle || [d.name, d.manufacturer, d.vidPid, d.busId].join(' ').toLowerCase().includes(needle));
  return (
    <DrillView dir={drill.dir} viewKey="devices">
      <div className="kp-page">
        <PageHeader
          title={`On ${host}`}
          actions={devices.length ? <SearchField aria-label="Search devices" placeholder="Search devices" shortcut="Ctrl K" size="md" value={q} onChange={setQ} width="var(--size-width-xs)" /> : undefined}
        />
        {unavailable ? (
          <Callout tone="danger" title="Devices cannot be shared">
            {unavailable}
          </Callout>
        ) : null}
        {devices.length === 0 ? (
          <EmptyState icon={Usb} title="Plug in a USB device to share it">
            Koppel lists every USB device on {host} here. Nothing is shared until you choose to.
          </EmptyState>
        ) : shown.length === 0 ? (
          <EmptyState title={`No devices match "${q}"`} />
        ) : (
          groups.map((group) => {
            const items = shown.filter((d) => group.states.includes(d.state));
            if (!items.length) return null;
            return (
              <CardGroup key={group.title} title={group.title} count={items.length}>
                {items.map((d) => {
                  const action = pending[d.busId];
                  const fault = failures[d.busId] ?? d.fault;
                  const holder = clientName(clients.data, d.attachedTo);
                  return (
                    <DeviceCard
                      key={d.busId}
                      id={d.busId}
                      name={d.name}
                      sub={deviceSub(d, holder, fault)}
                      subDanger={!!fault}
                      state={action ? <StatusDot tone="pending">{action.share ? 'Sharing' : 'Stopping'}</StatusDot> : <StateLabel states={states} state={deviceWord(d)} />}
                      control={<ShareSwitch device={d} holder={holder} bare />}
                      onOpen={() => drill.open(d.busId)}
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
