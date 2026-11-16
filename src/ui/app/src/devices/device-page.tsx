import { ActivityLog, Callout, EmptyState, Facts, PageHeader, StateLabel, Tile, TileStack } from '@heartwood/ui';
import { toEvents, useActivity } from '@/activity/activity-context';
import { states } from '@/brand';
import { clientName, useClients } from '@/clients/use-clients';
import { usePending } from '@/pending/pending-context';
import { ShareSwitch } from './device-switches';
import { deviceWord, kindOf } from './device-words';
import { since, useServerDevices } from './use-devices';

export interface Back {
  label: string;
  onPress: () => void;
}

/**
 * One of this machine's devices as a page of its own: the share switch, why it was not shared, its
 * facts and its history. The Devices page and the Overview both open it, each with its own way back.
 */
export function DevicePage({ busId, back }: { busId: string; back: Back }) {
  const device = useServerDevices().data?.devices.find((d) => d.busId === busId);
  const clients = useClients();
  const { failures } = usePending('serverDevices');
  const { items } = useActivity();
  if (!device) {
    return (
      <div className="kp-page">
        <PageHeader back={back} title="Device not found" />
        <EmptyState title="This device is no longer listed">It was unplugged, or the list has not loaded yet.</EmptyState>
      </div>
    );
  }
  const fault = failures[busId] ?? device.fault;
  const holder = clientName(clients.data, device.attachedTo);
  const history = toEvents(items, busId);
  const word = deviceWord(device);
  const kind = kindOf(device);
  return (
    <div className="kp-page">
      <PageHeader
        back={back}
        title={device.name}
        description={
          <span className="kp-line">
            {word === 'Attached' || word === 'Offline' ? <StateLabel states={states} state={word} /> : null}
            <span>
              {word === 'Attached'
                ? `by ${holder}${since(device.attachedSince)}`
                : word === 'Offline'
                  ? 'Unplugged. Koppel shares it again when it is plugged back in.'
                  : `${device.manufacturer ?? 'Unknown maker'}, ${kind ?? 'USB device'}`}
            </span>
          </span>
        }
        actions={<ShareSwitch device={device} holder={holder} />}
      />
      {fault ? (
        <div className="kp-text">
          <Callout tone="danger" title={`${device.name} is not shared`}>
            {fault}
          </Callout>
        </div>
      ) : null}
      <TileStack>
        <Tile title="Details" padded>
          <Facts
            label={`Details of ${device.name}`}
            items={[
              { label: 'Bus ID', value: device.busId, copy: true },
              { label: 'Maker', value: device.manufacturer ?? 'Unknown' },
              { label: 'VID:PID', value: device.vidPid || 'Unknown', mono: true },
              { label: 'Speed', value: device.speed || 'Unknown' },
              ...(device.serial ? [{ label: 'Serial', value: device.serial, mono: true }] : []),
              ...(device.driver ? [{ label: 'Driver', value: device.driver, mono: true }] : []),
              ...(kind ? [{ label: 'Kind', value: kind }] : []),
            ]}
          />
        </Tile>
        <ActivityLog
          states={states}
          events={history}
          meta={history.length ? String(history.length) : undefined}
          exportName={`${device.name}-history`}
        />
      </TileStack>
    </div>
  );
}
