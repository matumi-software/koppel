import type { ReactNode } from 'react';
import { StatusBadge, type DeviceState } from './StatusBadge';

export interface DeviceCardBodyProps {
  name: string;
  /** The product and maker as the device reports them, shown under the name. */
  vendor?: string;
  /** Vendor and product ID as four hex digits each, such as 0403:6001. */
  vidPid: string;
  busId: string;
  state: DeviceState;
  /** Where the device is, such as "Attached to bench-2 since 09:14". */
  where: ReactNode;
  /** The action for this role: a Switch on the server, a Button on the client. */
  action?: ReactNode;
  /** Caption at the start of the footer. */
  footnote?: ReactNode;
}

/** What a device card shows, inside DeviceCards (selectable). */
export function DeviceCardBody({ name, vendor, vidPid, busId, state, where, action, footnote }: DeviceCardBodyProps) {
  return (
    <>
      <div className="kp-device__head">
        <div className="kp-device__title">
          <h3 className="kp-device__name">{name}</h3>
          {vendor && <span className="kp-device__vendor">{vendor}</span>}
        </div>
        <StatusBadge state={state} />
      </div>
      <div className="kp-device__ids">
        {vidPid && <span className="kp-id">{vidPid}</span>}
        <span className="kp-id">bus {busId}</span>
      </div>
      <p className="kp-device__where">{where}</p>
      {(action || footnote) && (
        <div className="kp-device__foot">
          {footnote ? <span className="kp-switch__note">{footnote}</span> : <span />}
          {action}
        </div>
      )}
    </>
  );
}
