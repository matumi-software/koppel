import type { ReactNode } from 'react';
import { ActivityFeed, Caption, Chips, Drawer, DrawerSection, Holder, KeyValues, StatusBadge, type DeviceState } from '@koppel/react-ui';
import { toEntries, useActivity } from '@/activity/activity-context';
import type { DeviceView } from '@/api/usb';

export interface DeviceDrawerProps {
  device: DeviceView;
  state: DeviceState;
  /** Who has the device and what that means, such as "bench-2 has it". */
  holder: { state: DeviceState; title: string; detail: string };
  /** The control for this role: the Share switch, or Attach/Detach. */
  action: ReactNode;
  onClose: () => void;
}

/** A device's details in the aside: who holds it, its facts and its history this session. */
export function DeviceDrawer({ device, state, holder, action, onClose }: DeviceDrawerProps) {
  const { items } = useActivity();
  const facts = [
    { label: 'VID:PID', value: device.vidPid || 'unknown', mono: true },
    { label: 'Bus', value: device.busId, mono: true },
    { label: 'Speed', value: device.speed || 'unknown' },
    ...(device.serial ? [{ label: 'Serial', value: device.serial, mono: true }] : []),
    ...(device.driver ? [{ label: 'Driver', value: device.driver, mono: true }] : []),
    ...(device.interfaces && device.interfaces.length > 0 ? [{ label: 'Kind', value: <Chips items={device.interfaces} /> }] : []),
  ];

  return (
    <Drawer
      title={device.name}
      badge={<StatusBadge state={state} />}
      subtitle={device.manufacturer && <Caption tone="faint">{device.manufacturer}</Caption>}
      onClose={onClose}
    >
      <Holder state={holder.state} title={holder.title} detail={holder.detail} />
      <KeyValues items={facts} />
      <div>{action}</div>
      <DrawerSection title="History">
        <ActivityFeed entries={toEntries(items, device.busId)} empty="Nothing has happened to this device in this session." />
      </DrawerSection>
    </Drawer>
  );
}
