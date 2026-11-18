import { ActivityLog, Callout, CopyValue, EmptyState, Facts, PageHeader, StateLabel, Tile, TileStack } from '@heartwood/ui';
import { toEvents, useActivity } from '@/activity/activity-context';
import { states } from '@/brand';
import { usePending } from '@/pending/pending-context';
import type { Back } from './device-page';
import { AttachSwitch } from './device-switches';
import { kindOf, remoteEntries, remoteWord } from './device-words';
import { since, useClientDevices } from './use-devices';

/** The note a device that cannot be switched carries instead of a switch. */
const noteOf = (word: string, server: string) =>
  word === 'In use'
    ? `Another client of ${server} is using it. It can be attached when they detach it.`
    : word === 'Offline'
      ? `Unplugged on ${server}. It shows as Not attached again when it is plugged back in.`
      : null;

/**
 * One device of one server as a page: the attach switch, why it did not attach, its facts and its
 * history. The usbip command shows only for a device whose bus ID no earlier server also offers.
 */
export function RemoteDevicePage({ serverId, busId, back }: { serverId: string; busId: string; back: Back }) {
  const query = useClientDevices();
  const { failures } = usePending('clientDevices');
  const { items } = useActivity();
  const item = remoteEntries(query.data?.servers ?? []).find((r) => r.server.id === serverId && r.entry.device.busId === busId);
  if (!item) {
    return (
      <div className="kp-page">
        <PageHeader back={back} title="Device not found" />
        <EmptyState level={2} title="This device is no longer listed">It was unplugged, its server dropped it, or the list has not loaded yet.</EmptyState>
      </div>
    );
  }
  const { entry, server } = item;
  const device = entry.device;
  const fault = failures[item.key] ?? entry.fault;
  const word = remoteWord(entry, server.link);
  const note = noteOf(word, server.name);
  const history = toEvents(items, item.key);
  const kind = kindOf(device);
  const gateway = query.data?.gateway && !entry.attachedHere && device.state === 'shared' && item.unambiguous;
  return (
    <div className="kp-page">
      <PageHeader
        back={back}
        title={device.name}
        description={
          <span className="kp-line">
            <StateLabel states={states} state={word} />
            <span>{entry.attachedHere ? (entry.attachedVia ? `through the gateway for ${entry.attachedVia}${since(entry.attachedSince)}` : `from ${server.name}${since(entry.attachedSince)}`) : `on ${server.name}`}</span>
          </span>
        }
        actions={<AttachSwitch item={item} />}
      />
      {fault ? (
        <div className="kp-text">
          <Callout tone="danger" title={`${device.name} is not attached`}>
            {fault}
          </Callout>
        </div>
      ) : null}
      {note ? <p className="kp-muted">{note}</p> : null}
      <TileStack>
        <Tile title="Details" padded>
          <Facts
            label={`Details of ${device.name}`}
            items={[
              { label: 'Server', value: server.name },
              { label: 'Bus ID', value: device.busId, copy: true },
              { label: 'Maker', value: device.manufacturer ?? 'Unknown' },
              { label: 'VID:PID', value: device.vidPid || 'Unknown', mono: true },
              { label: 'Speed', value: device.speed || 'Unknown' },
              ...(device.serial ? [{ label: 'Serial', value: device.serial, mono: true }] : []),
              ...(kind ? [{ label: 'Kind', value: kind }] : []),
              ...(entry.attachedHere && entry.port !== null ? [{ label: 'Virtual port', value: String(entry.port), mono: true }] : []),
            ]}
          />
        </Tile>
        {gateway ? (
          <Tile title="With the USB/IP driver" padded>
            <div className="kp-stack-2">
              <p className="kp-muted">To attach it to the computer that runs this container, run this with usbip-win2.</p>
              <CopyValue value={`usbip attach -r 127.0.0.1 -b ${device.busId}`} label="usbip command" />
            </div>
          </Tile>
        ) : null}
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
