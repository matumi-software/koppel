import type { ReactNode } from 'react';
import { ActivityFeed, Button, Glyph, StatusBadge, type DeviceState } from '@koppel/react-ui';
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
  return (
    <>
      <div className="kp-drawer__head">
        <div style={{ display: 'grid', gap: 6, minWidth: 0, justifyItems: 'start' }}>
          <StatusBadge state={state} />
          <h2>{device.name}</h2>
          {device.manufacturer && <span className="kp-type-caption" style={{ color: 'var(--text-tertiary)' }}>{device.manufacturer}</span>}
        </div>
        <Button variant="ghost" aria-label="Close details" onPress={onClose}>
          Close
        </Button>
      </div>

      <div className="kp-holder" data-state={holder.state}>
        <Glyph state={holder.state === 'pending' ? 'pending' : holder.state} />
        <div style={{ display: 'grid', gap: 2, minWidth: 0 }}>
          <span className="kp-type-label">{holder.title}</span>
          <span className="kp-type-caption">{holder.detail}</span>
        </div>
      </div>

      <dl className="kp-kv">
        <dt>VID:PID</dt>
        <dd className="kp-type-mono">{device.vidPid || 'unknown'}</dd>
        <dt>Bus</dt>
        <dd className="kp-type-mono">{device.busId}</dd>
        <dt>Speed</dt>
        <dd>{device.speed || 'unknown'}</dd>
        {device.serial && (
          <>
            <dt>Serial</dt>
            <dd className="kp-type-mono">{device.serial}</dd>
          </>
        )}
        {device.driver && (
          <>
            <dt>Driver</dt>
            <dd className="kp-type-mono">{device.driver}</dd>
          </>
        )}
        {device.interfaces && device.interfaces.length > 0 && (
          <>
            <dt>Kind</dt>
            <dd>
              <span className="kp-chips">
                {device.interfaces.map((name) => (
                  <span key={name} className="kp-chip">
                    {name}
                  </span>
                ))}
              </span>
            </dd>
          </>
        )}
      </dl>

      <div>{action}</div>

      <hr style={{ border: 0, height: 1, background: 'var(--border-default)', margin: 0, width: '100%' }} />
      <h3 className="kp-type-heading" style={{ margin: 0 }}>
        History
      </h3>
      <ActivityFeed entries={toEntries(items, device.busId)} empty="Nothing has happened to this device in this session." />
    </>
  );
}
