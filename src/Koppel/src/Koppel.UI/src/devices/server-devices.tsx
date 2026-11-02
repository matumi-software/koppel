import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { DeviceCard, Switch } from '@koppel/react-ui';
import { getServerDevices, setShared, type ServerDevices } from '@/api/usb';
import { useHubEvent } from '@/hub/hub-context';
import { usePending } from '@/shared/use-pending';

const key = ['server-devices'] as const;

/**
 * Server role: the devices plugged into this machine. Sharing is pending until DevicesChanged shows
 * the kernel has (or has not) taken the device, or reports why it refused.
 */
export function ServerDevicesView({ locked }: { locked: boolean }) {
  const queryClient = useQueryClient();
  const devices = useQuery({ queryKey: key, queryFn: getServerDevices });
  const { pending, failures, request, judge } = usePending<boolean>();

  const onDevicesChanged = useCallback(
    (next: ServerDevices) => {
      queryClient.setQueryData(key, next);
      judge((busId, share) => {
        const device = next.devices.find((d) => d.busId === busId);
        if (device?.fault) return { failed: device.fault };
        const isShared = device ? device.state !== 'local' : false;
        return isShared === share ? 'confirmed' : 'waiting';
      });
    },
    [queryClient, judge],
  );
  useHubEvent('DevicesChanged', onDevicesChanged);

  if (!devices.data) {
    return <p className="app-note">Loading devices…</p>;
  }

  const { unavailable } = devices.data;
  return (
    <div className="app-stack">
      <h1 className="kp-type-display app-title">Devices on this machine</h1>
      {unavailable && <p className="app-error">{unavailable}</p>}
      {devices.data.devices.length === 0 && !unavailable && (
        <p className="app-note app-flush">Plug a device into this machine to share it.</p>
      )}
      <div className="app-grid">
        {devices.data.devices.map((device) => {
          const target = pending[device.busId];
          const isShared = device.state !== 'local';
          return (
            <DeviceCard
              key={device.busId}
              name={device.name}
              vidPid={device.vidPid || 'unplugged'}
              busId={device.busId}
              state={target !== undefined ? 'pending' : device.state}
              where={
                device.state === 'attached' ? (
                  <>
                    Attached to <b>{device.attachedTo ?? 'a client'}</b>
                  </>
                ) : device.state === 'offline' ? (
                  'Unplugged. It is shared again when it comes back.'
                ) : (
                  device.speed
                )
              }
              action={
                <Switch
                  label="Share this device"
                  confirmed={isShared}
                  pendingTarget={target}
                  failure={failures[device.busId] ?? device.fault ?? undefined}
                  isDisabled={locked || !!unavailable || device.state === 'attached'}
                  onRequestChange={(share) => request(device.busId, share, () => setShared(device.busId, share))}
                />
              }
            />
          );
        })}
      </div>
    </div>
  );
}
