import { Glyph } from './Activity';
import { Button } from './Button';

/** The UI's live link to the machine it runs on: up, coming back, or away through every retry. */
export type LinkState = 'live' | 'reconnecting' | 'down';

export interface StatusCardProps {
  /** The UI's live link to this machine. */
  state: LinkState;
  /** The machine this UI runs on, as the server reports it. */
  machineName: string;
  /** The Koppel version it runs, as it should read, such as v0.1.1. Left out until it is known. */
  version?: string;
  /** One more line about the machine, such as what a client is linked to. `pending` when it is not as it should be. */
  detail?: { text: string; tone?: 'pending' };
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
 * version it runs, one more line such as what a client is linked to, and Sign out. It is not a live
 * region: link changes are announced once, by the Announcer.
 */
export function StatusCard({ state, machineName, version, detail, onRetry, onSignOut, signOutError }: StatusCardProps) {
  return (
    <div className="kp-status-card" data-state={state}>
      <div className="kp-status-card__head">
        <span className="kp-status-card__state">
          <Glyph state={glyphs[state]} />
          {words[state]}
        </span>
        {version && (
          <span className="kp-status-card__version">
            <span className="kp-sr-only">Koppel </span>
            {version}
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
        {detail && (
          <p className="kp-status-card__line" data-tone={detail.tone}>
            {detail.text}
          </p>
        )}
        {signOutError && (
          <p className="kp-status-card__line" data-tone="fault">
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

/** The live link as a dot, for the phone top bar. The state is in words in its name. */
export function LinkDot({ state }: { state: LinkState }) {
  return (
    <span className="kp-row" role="img" aria-label={words[state]}>
      <Glyph state={glyphs[state]} />
    </span>
  );
}
