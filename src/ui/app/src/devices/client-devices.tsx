import { useState } from 'react';
import { Button, Caption, CopyValue, DeviceCardBody, DeviceCards, DeviceGroup, Note, Row, Stack, type DeviceState } from '@koppel/react-ui';
import { useQuery } from '@tanstack/react-query';
import type { ClientDeviceView } from '@/api/usb';
import { Frame } from '@/shell/frame';
import { goTo, useFrame } from '@/shell/frame-context';
import { usePending } from '@/pending/pending-context';
import { DeviceDrawer } from './device-drawer';
import { hostOf, since, useClientDevices } from './use-devices';
import { connectionOptions } from '@/queries';

/**
 * Client role: the devices the server shares, grouped by what you can do with them. Attach and detach
 * are pending until ClientDevicesChanged shows the device attached here (or not), or why it failed.
 */
export function ClientDevicesPage() {
  const { locked } = useFrame();
  const connection = useQuery(connectionOptions());
  const server = hostOf(connection.data?.serverUrl) ?? 'the server';
  const [selected, setSelected] = useState<string | null>(null);
  const { pending, failures, request } = usePending('clientDevices');
  const query = useClientDevices();
  const data = query.data;
  const entries = data?.devices ?? [];

  const stateOf = (e: ClientDeviceView): DeviceState => (e.device.busId in pending ? 'pending' : e.device.state);
  const takenElsewhere = (e: ClientDeviceView) => e.device.state === 'attached' && !e.attachedHere;
  const action = (e: ClientDeviceView) => {
    const busy = e.device.busId in pending;
    return e.attachedHere ? (
      <Button variant="secondary" isSending={busy} isDisabled={locked} onPress={() => request({ busId: e.device.busId, attach: false })}>
        {busy ? 'Detaching' : 'Detach'}
      </Button>
    ) : (
      <Button
        variant="primary"
        isSending={busy}
        isDisabled={locked || takenElsewhere(e) || e.device.state === 'offline' || !!data?.unavailable}
        onPress={() => request({ busId: e.device.busId, attach: true })}
      >
        {busy ? 'Attaching' : 'Attach'}
      </Button>
    );
  };
  const where = (e: ClientDeviceView) =>
    e.attachedHere && e.attachedVia ? (
      <>
        Attached through the gateway for <b>{e.attachedVia}</b>
        {since(e.attachedSince)}
      </>
    ) : e.attachedHere ? (
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
        current.attachedHere && current.attachedVia
          ? { state: 'attached', title: 'Attached through the gateway', detail: `The USB/IP driver on ${current.attachedVia} has it${since(current.attachedSince)}. Detach here unplugs it there.` }
          : current.attachedHere
            ? { state: 'attached', title: 'Attached here', detail: `On virtual port ${current.port}${since(current.attachedSince)}.` }
            : takenElsewhere(current)
              ? { state: 'attached', title: 'In use elsewhere', detail: 'Another client has it. Ask there to detach it.' }
              : { state: 'shared', title: 'Free to attach', detail: `${server} shares it with registered clients.` }
      }
      action={
        <>
          {action(current)}
          {data?.gateway && !current.attachedHere && current.device.state === 'shared' && (
            <Stack gap="sm">
              <Caption>Or attach it to the computer that runs this container, with usbip-win2:</Caption>
              <CopyValue value={`usbip attach -r 127.0.0.1 -b ${current.device.busId}`} wrap />
            </Stack>
          )}
        </>
      }
      onClose={() => setSelected(null)}
    />
  ) : undefined;

  const link = data?.link;
  return (
    <Frame title={`Devices on ${server}`} lede="Attach a device to use it here as if it were plugged in." aside={aside} asideKey={current?.device.busId}>
      {link === 'unconfigured' && (
        <Row>
          <Note>Set up the server connection to see its devices.</Note>
          <Button variant="secondary" onPress={() => goTo('configuration')}>
            Server connection
          </Button>
        </Row>
      )}
      {link === 'connecting' && <Note>Connecting to {server}…</Note>}
      {link === 'error' && <Note tone="error">Cannot reach {server}: {data?.linkMessage}</Note>}
      {data?.unavailable && <Note tone="error">{data.unavailable}</Note>}
      {link === 'connected' && entries.length === 0 && <Note>{server} is not sharing any devices.</Note>}
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
