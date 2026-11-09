import type { ReactNode } from 'react';

export type LinkState = 'live' | 'reconnecting' | 'down';

export interface LinkBannerProps {
  state: LinkState;
  /** The server this UI is linked to. */
  serverName: string;
  /** Shown as a Retry button when the link is down. */
  retry?: ReactNode;
}

/** Shows whether changes can be confirmed right now. */
export function LinkBanner({ state, serverName, retry }: LinkBannerProps) {
  if (state === 'live') {
    return (
      <div className="kp-link" data-state="live" role="status">
        <span className="kp-link__dot" />
        <span>Live</span>
        <span className="kp-link__detail">{serverName}</span>
      </div>
    );
  }

  if (state === 'reconnecting') {
    return (
      <div className="kp-link" data-state="reconnecting" role="status">
        <span className="kp-link__dot" />
        <span>Reconnecting.</span>
        <span className="kp-link__detail">Changes are paused until the link is back.</span>
      </div>
    );
  }

  return (
    <div className="kp-link" data-state="down" role="alert">
      <span className="kp-link__dot" />
      <span>Disconnected.</span>
      <span className="kp-link__detail">Check that {serverName} is running, then retry.</span>
      {retry}
    </div>
  );
}
