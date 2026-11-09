import { Glyph } from './Activity';
import { Button } from './Button';
import type { LinkState } from './LinkBanner';

/** A client's link to the server it attaches from, as the client reports it. */
export type ServerLinkState = 'unconfigured' | 'connecting' | 'connected' | 'error';

export interface StatusCardProps {
  /** The UI's live link to this machine. */
  state: LinkState;
  /** The machine this UI runs on, as the server reports it. */
  machineName: string;
  /** The Koppel version it runs, such as 0.1.1. Left out until it is known. */
  version?: string;
  /** For a client: the server it attaches from. Left out on a server. */
  server?: { link: ServerLinkState; name?: string | null };
  /** Shown as Retry while the link is down. */
  onRetry?: () => void;
  onSignOut: () => void;
  /** Why the last sign out failed, such as "the server answered 500.". */
  signOutError?: string;
}

const words: Record<LinkState, string> = { live: 'Live', reconnecting: 'Reconnecting', down: 'Disconnected' };
const glyphs = { live: 'live', reconnecting: 'pending', down: 'fault' } as const;

/**
 * The card at the foot of the sidebar: whether the live link is up, this machine and the Koppel
 * version it runs, what a client is linked to, and Sign out.
 */
export function StatusCard({ state, machineName, version, server, onRetry, onSignOut, signOutError }: StatusCardProps) {
  return (
    <div className="kp-status-card" data-state={state}>
      <div className="kp-status-card__head">
        {/* Keyed by state so a screen reader hears the change, and the alert role only when the link is down. */}
        <span key={state} className="kp-status-card__state" role={state === 'down' ? 'alert' : 'status'}>
          <Glyph state={glyphs[state]} />
          {words[state]}
        </span>
        {version && (
          <span className="kp-status-card__version">
            <span className="kp-sr-only">Koppel </span>
            {/^\d/.test(version) ? `v${version}` : version}
          </span>
        )}
      </div>

      <div className="kp-status-card__body">
        <span className="kp-status-card__name" title={machineName}>
          {machineName}
        </span>
        {state === 'reconnecting' && (
          <p className="kp-status-card__line" data-tone="pending">
            Changes are paused until the link is back.
          </p>
        )}
        {state === 'down' && (
          <p className="kp-status-card__line" data-tone="fault">
            Check that {machineName} is running, then retry.
          </p>
        )}
        {server && (
          <p className="kp-status-card__line" data-tone={server.link === 'connected' ? undefined : 'pending'}>
            {serverLine(server.link, server.name)}
          </p>
        )}
        {signOutError && (
          <p className="kp-status-card__line" data-tone="fault" role="status">
            Not signed out: {signOutError}
          </p>
        )}
      </div>

      <div className="kp-status-card__actions">
        {state === 'down' && onRetry && (
          <Button variant="secondary" onPress={onRetry}>
            Retry
          </Button>
        )}
        <Button variant="ghost" onPress={onSignOut}>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" />
            <path d="m16 17 5-5-5-5" />
            <path d="M21 12H9" />
          </svg>
          Sign out
        </Button>
      </div>
    </div>
  );
}

function serverLine(link: ServerLinkState, name: string | null | undefined) {
  switch (link) {
    case 'connected':
      return `Linked to ${name ?? 'the server'}.`;
    case 'unconfigured':
      return 'No server connection yet.';
    case 'connecting':
      return `Connecting to ${name ?? 'the server'}…`;
    default:
      return `Cannot reach ${name ?? 'the server'}.`;
  }
}
