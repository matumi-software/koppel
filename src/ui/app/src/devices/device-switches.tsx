import { useState } from 'react';
import { Switch } from '@matumi/ui';
import type { DeviceView } from '@/api/usb';
import { usePending } from '@/pending/pending-context';
import { ConfirmDialog } from '@/shared/confirm-dialog';
import { useFrame } from '@/shell/frame-context';
import type { RemoteEntry } from './device-words';
import { useClientDevices, useServerDevices } from './use-devices';


/**
 * Sharing one of this machine's devices, under the confirm rule: the switch shows the asked-for value
 * with a spinner until DevicesChanged says the kernel took it. Stopping a device a client has attached
 * asks first, since that client loses it. `bare` leaves the visible label out where a state line
 * beside the switch already says it.
 */
export function ShareSwitch({ device, holder, bare }: { device: DeviceView; holder?: string | null; bare?: boolean }) {
  const { locked } = useFrame();
  const { pending, request } = usePending('serverDevices');
  const unavailable = useServerDevices().data?.unavailable;
  const [confirming, setConfirming] = useState(false);
  const action = pending[device.busId];
  const on = action ? action.share : device.state !== 'local';
  const send = (share: boolean) => request({ busId: device.busId, share });
  return (
    <>
      <Switch
        aria-label={bare ? `Share ${device.name}` : undefined}
        isSelected={on}
        isPending={!!action}
        isDisabled={locked || !!unavailable}
        onChange={(next) => (!next && device.state === 'attached' ? setConfirming(true) : send(next))}
      >
        {bare ? null : 'Shared'}
      </Switch>
      {confirming ? (
        <ConfirmDialog title={`Stop sharing ${device.name}?`} commit="Stop sharing" danger onConfirm={() => send(false)} onClose={() => setConfirming(false)}>
          {holder ?? 'A client'} is using it and loses it at once. You can share it again later.
        </ConfirmDialog>
      ) : null}
    </>
  );
}

/**
 * Attaching or detaching a device of a server, under the confirm rule. A device another client has, or
 * one that is unplugged, has no switch: there is nothing to turn.
 */
export function AttachSwitch({ item, bare }: { item: RemoteEntry; bare?: boolean }) {
  const { locked } = useFrame();
  const { pending, request } = usePending('clientDevices');
  const unavailable = useClientDevices().data?.unavailable;
  const { entry, server } = item;
  const action = pending[item.key];
  if (!entry.attachedHere && !action && (entry.device.state === 'offline' || entry.device.state === 'attached')) return null;
  const on = action ? action.attach : entry.attachedHere;
  return (
    <Switch
      aria-label={bare ? `Attach ${entry.device.name}` : undefined}
      isSelected={on}
      isPending={!!action}
      isDisabled={locked || !!unavailable}
      onChange={(attach) => request({ serverId: server.id, busId: entry.device.busId, attach })}
    >
      {bare ? null : 'Attached'}
    </Switch>
  );
}
