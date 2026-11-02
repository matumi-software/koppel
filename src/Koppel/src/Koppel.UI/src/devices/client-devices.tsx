import { useCallback, useState } from 'react';
import { Button, DeviceCardBody, DeviceCards, DeviceGroup, type DeviceState } from '@koppel/react-ui';
import { useQuery } from '@tanstack/react-query';
import { getConnection } from '@/api/client';
import { attachDevice, detachDevice, type ClientDeviceView, type ClientDevices } from '@/api/usb';
import { Frame } from '@/shell/frame';
import { SideInfo } from '@/shared/side-info';
import { useFrame } from '@/shell/frame-context';
import { usePending } from '@/shared/use-pending';
import { DeviceDrawer } from './device-drawer';
import { hostOf, since, useClientDevices } from './use-devices';

/** Attaching can take a while: TLS, token, import and the kernel. */
const attachTimeoutMs = 30_000;

/**
 * Client role: the devices the server shares, grouped by what you can do with them. Attach and detach
 * are pending until ClientDevicesChanged shows the device attached here (or not), or why it failed.
 */
export function ClientDevicesPage() {
  const { locked, go } = useFrame();
  const connection = useQuery({ queryKey: ['connection'], queryFn: getConnection });
  const server = hostOf(connection.data?.serverUrl) ?? 'the server';
  const [selected, setSelected] = useState<string | null>(null);
  const describe = useCallback((busId: string, attach: boolean) => ({ text: `${attach ? 'Attach' : 'Detach'} ${busId}`, subject: busId }), []);
  const { pending, failures, request, judge } = usePending<boolean>({ timeoutMs: attachTimeoutMs, describe });

  const onPush = useCallback(
    (next: ClientDevices) =>
      judge((busId, attach) => {
        const entry = next.devices.find((d) => d.device.busId === busId);
        if (entry?.fault) return { failed: entry.fault };
        return (entry?.attachedHere ?? false) === attach ? 'confirmed' : 'waiting';
      }),
    [judge],
  );
  const query = useClientDevices(true, onPush);
  const data = query.data;
  const entries = data?.devices ?? [];

  const stateOf = (e: ClientDeviceView): DeviceState => (e.device.busId in pending ? 'pending' : e.device.state);
  const takenElsewhere = (e: ClientDeviceView) => e.device.state === 'attached' && !e.attachedHere;
  const action = (e: ClientDeviceView) => {
    const busy = e.device.busId in pending;
    return e.attachedHere ? (
      <Button variant="secondary" isSending={busy} isDisabled={locked} onPress={() => request(e.device.busId, false, () => detachDevice(e.device.busId))}>
        {busy ? 'Detaching' : 'Detach'}
      </Button>
    ) : (
      <Button
        variant="primary"
        isSending={busy}
        isDisabled={locked || takenElsewhere(e) || e.device.state === 'offline' || !!data?.unavailable}
        onPress={() => request(e.device.busId, true, () => attachDevice(e.device.busId))}
      >
        {busy ? 'Attaching' : 'Attach'}
      </Button>
    );
  };
  const where = (e: ClientDeviceView) =>
    e.attachedHere ? (
      <>
        Attached here on virtual port <b>{e.port}</b>
        {since(e.attachedSince)}
      </>
    ) : takenElsewhere(e) ? (
      'In use by another client'
    ) : e.device.state === 'offline' ? (
      `Unplugged on ${server}`
    ) : (
      e.device.speed
    );

  const groups: { title: string; items: ClientDeviceView[] }[] = [
    { title: 'Waiting for the server', items: entries.filter((e) => e.device.busId in pending) },
    { title: 'Attached here', items: entries.filter((e) => !(e.device.busId in pending) && e.attachedHere) },
    { title: 'Free to attach', items: entries.filter((e) => !(e.device.busId in pending) && !e.attachedHere && e.device.state === 'shared') },
    { title: 'Not available', items: entries.filter((e) => !(e.device.busId in pending) && !e.attachedHere && e.device.state !== 'shared') },
  ];

  const current = entries.find((e) => e.device.busId === selected);
  const aside = current ? (
    <DeviceDrawer
      device={current.device}
      state={stateOf(current)}
      holder={
        current.attachedHere
          ? { state: 'attached', title: 'Attached here', detail: `On virtual port ${current.port}${since(current.attachedSince)}.` }
          : takenElsewhere(current)
            ? { state: 'attached', title: 'In use elsewhere', detail: 'Another client has it. Ask there to detach it.' }
            : { state: 'shared', title: 'Free to attach', detail: `${server} shares it with registered clients.` }
      }
      action={action(current)}
      onClose={() => setSelected(null)}
    />
  ) : <SideInfo />;

  const link = data?.link;
  return (
    <Frame title={`Devices on ${server}`} lede="Attach a device to use it here as if it were plugged in." aside={aside} asideWide={!!current}>
      {link === 'unconfigured' && (
        <div className="kp-row">
          <p className="kp-note">Set up the server connection to see its devices.</p>
          <Button variant="secondary" onPress={() => go('connection')}>
            Server connection
          </Button>
        </div>
      )}
      {link === 'connecting' && <p className="kp-note">Connecting to {server}…</p>}
      {link === 'error' && <p className="kp-error">Cannot reach {server}: {data?.linkMessage}</p>}
      {data?.unavailable && <p className="kp-error">{data.unavailable}</p>}
      {link === 'connected' && entries.length === 0 && <p className="kp-note">{server} is not sharing any devices.</p>}
      {groups
        .filter((g) => g.items.length > 0)
        .map((group) => (
          <DeviceGroup key={group.title} title={group.title} count={group.items.length}>
            <DeviceCards
              label={group.title}
              items={group.items}
              getKey={(e) => e.device.busId}
              getName={(e) => e.device.name}
              getState={stateOf}
              selectedKey={selected}
              onSelectionChange={setSelected}
            >
              {(e) => {
                const failure = failures[e.device.busId] ?? e.fault;
                return (
                  <DeviceCardBody
                    name={e.device.name}
                    vendor={e.device.manufacturer ?? undefined}
                    vidPid={e.device.vidPid}
                    busId={e.device.busId}
                    state={stateOf(e)}
                    where={where(e)}
                    footnote={failure ? `Not done: ${failure}` : undefined}
                    action={action(e)}
                  />
                );
              }}
            </DeviceCards>
          </DeviceGroup>
        ))}
    </Frame>
  );
}
