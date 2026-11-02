import type { ReactNode } from 'react';

export type GlyphState = 'shared' | 'attached' | 'pending' | 'offline' | 'local' | 'live' | 'fault';

/** The state shapes on their own: circle, square, stripe, cross, ring. Decorative; say the state in words too. */
export function Glyph({ state }: { state: GlyphState }) {
  return <span className="kp-glyph" data-state={state} aria-hidden="true" />;
}

export interface ActivityEntry {
  id: string;
  state: GlyphState;
  /** A short lead word, such as "Saved." or "Not saved.". */
  lead?: string;
  text: ReactNode;
  /** When, already formatted, such as 10:42:18. */
  time: string;
}

/** Changes as the server confirms or rejects them, newest first. A live log for screen readers. */
export function ActivityFeed({ entries, empty }: { entries: readonly ActivityEntry[]; empty?: ReactNode }) {
  if (entries.length === 0) {
    return <p className="kp-note">{empty ?? 'Nothing yet.'}</p>;
  }
  return (
    <ol className="kp-feed" role="log" aria-live="polite">
      {entries.map((entry) => (
        <li key={entry.id} className="kp-feed__item" data-state={entry.state}>
          <Glyph state={entry.state} />
          <div>
            <p className="kp-feed__text">
              {entry.lead && <b>{entry.lead} </b>}
              {entry.text}
            </p>
            <span className="kp-feed__time">{entry.time}</span>
          </div>
        </li>
      ))}
    </ol>
  );
}
