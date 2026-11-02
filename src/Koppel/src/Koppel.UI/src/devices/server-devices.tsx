import { useCallback, useState } from 'react';
import { ActivityFeed, DeviceCardBody, DeviceCards, DeviceGroup, Switch, type DeviceState } from '@koppel/react-ui';
import { toEntries, useActivity } from '@/activity/activity-context';
import { setShared, type DeviceView, type ServerDevices } from '@/api/usb';
import { Frame } from '@/shell/frame';
import { useMachineName } from '@/shared/use-machine-name';
import { useFrame } from '@/shell/frame-context';
import { usePending } from '@/shared/use-pending';
import { DeviceDrawer } from './device-drawer';
import { since, useServerDevices } from './use-devices';

/**
 * Server role: the devices on this machine, grouped by what they need. Sharing is pending until
 * DevicesChanged shows the kernel took (or released) the device, or why it refused.
 */
export function ServerDevicesPage() {
  const host = useMachineName();
  const { locked } = useFrame();
  const { items } = useActivity();
  const [selected, setSelected] = useState<string | null>(null);
  const describe = useCallback((busId: string, share: boolean) => ({ text: `${share ? 'Share' : 'Stop sharing'} ${busId}`, subject: busId }), []);
  const { pending, failures, request, judge } = usePending<boolean>({ describe });

  const onPush = useCallback(
    (next: ServerDevices) =>
      judge((busId, share) => {
        const device = next.devices.find((d) => d.busId === busId);
        if (device?.fault) return { failed: device.fault };
        return (device ? device.state !== 'local' : false) === share ? 'confirmed' : 'waiting';
      }),
    [judge],
  );
  const query = useServerDevices(true, onPush);
  const data = query.data;

  const stateOf = (d: DeviceView): DeviceState => (d.busId in pending ? 'pending' : d.state);
  const shareSwitch = (d: DeviceView) => (
    <Switch
      label="Share"
      confirmed={d.state !== 'local'}
      pendingTarget={pending[d.busId]}
      failure={failures[d.busId] ?? d.fault ?? undefined}
      isDisabled={locked || !!data?.unavailable || d.state === 'attached'}
      onRequestChange={(share) => request(d.busId, share, () => setShared(d.busId, share))}
    />
  );
  const where = (d: DeviceView) =>
    d.state === 'attached' ? (
      <>
        Attached to <b>{d.attachedTo ?? 'a client'}</b>
        {since(d.attachedSince)}
      </>
    ) : d.state === 'offline' ? (
      'Unplugged. It is shared again when it comes back.'
    ) : d.state === 'shared' ? (
      'Ready for any client to attach'
    ) : (
      'Only on this machine'
    );

  const devices = data?.devices ?? [];
  const groups: { title: string; items: DeviceView[] }[] = [
    { title: 'Waiting for the server', items: devices.filter((d) => d.busId in pending) },
    { title: 'In use', items: devices.filter((d) => !(d.busId in pending) && d.state === 'attached') },
    { title: 'Shared and free', items: devices.filter((d) => !(d.busId in pending) && d.state === 'shared') },
    { title: 'Not shared or offline', items: devices.filter((d) => !(d.busId in pending) && (d.state === 'local' || d.state === 'offline')) },
  ];

  const current = devices.find((d) => d.busId === selected);
  const aside = current ? (
    <DeviceDrawer
      device={current}
      state={stateOf(current)}
      holder={
        current.state === 'attached'
          ? { state: 'attached', title: `${current.attachedTo ?? 'A client'} has it`, detail: `Attached${since(current.attachedSince)}. Ask there to detach it.` }
          : current.state === 'shared'
            ? { state: 'shared', title: 'Free', detail: 'Any registered client can attach it.' }
            : current.state === 'offline'
              ? { state: 'offline', title: 'Unplugged', detail: 'It is shared again when it comes back.' }
              : { state: 'local', title: 'Only on this machine', detail: 'Share it to let clients attach it.' }
      }
      action={shareSwitch(current)}
      onClose={() => setSelected(null)}
    />
  ) : (
    <>
      <h2 className="kp-type-heading" style={{ margin: 0 }}>
        Activity
      </h2>
      <ActivityFeed entries={toEntries(items)} empty="Changes show here when the server confirms or rejects them." />
    </>
  );

  return (
    <Frame title="Devices on this machine" lede={`Plug a device into ${host} to share it. Changes show once the server confirms them.`} aside={aside} asideWide={!!current}>
      {data?.unavailable && <p className="kp-error">{data.unavailable}</p>}
      {query.isPending && <p className="kp-note">Loading devices…</p>}
      {data && devices.length === 0 && !data.unavailable && <p className="kp-note">No USB devices found on this machine.</p>}
      {groups
        .filter((g) => g.items.length > 0)
        .map((group) => (
          <DeviceGroup key={group.title} title={group.title} count={group.items.length}>
            <DeviceCards
              label={group.title}
              items={group.items}
              getKey={(d) => d.busId}
              getName={(d) => d.name}
              getState={stateOf}
              selectedKey={selected}
              onSelectionChange={setSelected}
            >
              {(d) => (
                <DeviceCardBody
                  name={d.name}
                  vendor={d.manufacturer ?? undefined}
                  vidPid={d.vidPid}
                  busId={d.busId}
                  state={stateOf(d)}
                  where={where(d)}
                  footnote={d.busId in pending ? 'Saving…' : (failures[d.busId] ?? d.fault) ? 'Not saved' : d.speed}
                  action={shareSwitch(d)}
                />
              )}
            </DeviceCards>
          </DeviceGroup>
        ))}
    </Frame>
  );
}
