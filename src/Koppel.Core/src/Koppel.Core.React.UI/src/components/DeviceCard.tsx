import type { ReactNode } from 'react';
import { StatusBadge, type DeviceState } from './StatusBadge';

export interface DeviceCardProps {
  name: string;
  /** Vendor and product ID as four hex digits each, such as 0403:6001. */
  vidPid: string;
  busId: string;
  state: DeviceState;
  /** Where the device is, such as "Plugged into lab-rack-3". */
  where: ReactNode;
  /** The action for this role: a Switch on the server, a Button on the client. */
  action: ReactNode;
  /** Caption at the start of the footer. */
  footnote?: string;
  isSelected?: boolean;
}

/** One USB device: what it is, where it is, its state and the action for this role. */
export function DeviceCard({ name, vidPid, busId, state, where, action, footnote, isSelected }: DeviceCardProps) {
  return (
    <article className="kp-device" data-selected={isSelected ? 'true' : undefined}>
      <div className="kp-device__head">
        <h3 className="kp-device__name">{name}</h3>
        <StatusBadge state={state} />
      </div>
      <div className="kp-device__ids">
        <span className="kp-id">{vidPid}</span>
        <span className="kp-id">bus {busId}</span>
      </div>
      <p className="kp-device__where">{where}</p>
      <div className="kp-device__foot">
        {footnote ? <span className="kp-switch__note">{footnote}</span> : <span />}
        {action}
      </div>
    </article>
  );
}
