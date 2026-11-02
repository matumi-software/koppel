import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { Button, DeviceCard } from '@koppel/react-ui';
import { attachDevice, detachDevice, getClientDevices, type ClientDevices } from '@/api/usb';
import { useHubEvent } from '@/hub/hub-context';
import { usePending } from '@/shared/use-pending';

const key = ['client-devices'] as const;

/** Attaching can take a while: TLS, token, import and the kernel. */
const attachTimeoutMs = 30_000;

/**
 * Client role: the devices the server shares. Attach and detach are pending until
 * ClientDevicesChanged shows the device attached here (or not), or reports why it failed.
 */
export function ClientDevicesView({ locked }: { locked: boolean }) {
  const queryClient = useQueryClient();
  const devices = useQuery({ queryKey: key, queryFn: getClientDevices });
  const { pending, failures, request, judge } = usePending<boolean>(attachTimeoutMs);

  const onChanged = useCallback(
    (next: ClientDevices) => {
      queryClient.setQueryData(key, next);
      judge((busId, attach) => {
        const entry = next.devices.find((d) => d.device.busId === busId);
        if (entry?.fault) return { failed: entry.fault };
        return (entry?.attachedHere ?? false) === attach ? 'confirmed' : 'waiting';
      });
    },
    [queryClient, judge],
  );
  useHubEvent('ClientDevicesChanged', onChanged);

  if (!devices.data) {
    return <p className="app-note">Loading devices…</p>;
  }

  const { link, linkMessage, unavailable } = devices.data;
  return (
    <div className="app-stack">
      <h1 className="kp-type-display app-title">Devices on the server</h1>
      {link === 'unconfigured' && <p className="app-note app-flush">Set up the server connection in Settings to see its devices.</p>}
      {link === 'connecting' && <p className="app-note app-flush">Connecting to the server…</p>}
      {link === 'error' && <p className="app-error">Cannot reach the server: {linkMessage}</p>}
      {unavailable && <p className="app-error">{unavailable}</p>}
      {link === 'connected' && devices.data.devices.length === 0 && (
        <p className="app-note app-flush">The server is not sharing any devices.</p>
      )}
      <div className="app-grid">
        {devices.data.devices.map(({ device, attachedHere, port, fault }) => {
          const target = pending[device.busId];
          const busy = target !== undefined;
          const takenElsewhere = device.state === 'attached' && !attachedHere;
          const failure = failures[device.busId] ?? fault;
          return (
            <DeviceCard
              key={device.busId}
              name={device.name}
              vidPid={device.vidPid}
              busId={device.busId}
              state={busy ? 'pending' : device.state}
              where={attachedHere ? <>Attached here on virtual port <b>{port}</b></> : takenElsewhere ? 'In use by another client' : device.speed}
              footnote={failure ? `Not done: ${failure}` : undefined}
              action={
                attachedHere ? (
                  <Button
                    variant="secondary"
                    isSending={busy}
                    isDisabled={locked}
                    onPress={() => request(device.busId, false, () => detachDevice(device.busId))}
                  >
                    {busy ? 'Detaching' : 'Detach'}
                  </Button>
                ) : (
                  <Button
                    variant="primary"
                    isSending={busy}
                    isDisabled={locked || takenElsewhere || !!unavailable}
                    onPress={() => request(device.busId, true, () => attachDevice(device.busId))}
                  >
                    {busy ? 'Attaching' : 'Attach'}
                  </Button>
                )
              }
            />
          );
        })}
      </div>
    </div>
  );
}
