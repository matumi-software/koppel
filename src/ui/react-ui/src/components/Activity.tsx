import type { ReactNode } from 'react';

export type GlyphState = 'shared' | 'attached' | 'pending' | 'offline' | 'local' | 'live' | 'fault';

/** The state shapes on their own: circle, square, stripe, cross, ring. Decorative; say the state in words too. */
export function Glyph({ state }: { state: GlyphState }) {
  return <span className="kp-glyph" data-state={state} aria-hidden="true" />;
}

export interface ActivityEntry {
  id: string;
  state: GlyphState;
  /** A short lead word, such as "Waiting." or "Not done.". */
  lead?: string;
  text: ReactNode;
  /** Why it failed, shown under the text. */
  detail?: ReactNode;
  /** When, already formatted, such as 10:42:18. */
  time: string;
  /** What it is about, such as device or client, shown beside the time. */
  kind?: string;
}

/** Changes as the server confirms or rejects them, newest first. The Announcer says what a screen reader needs to hear. */
export function ActivityFeed({ entries, empty }: { entries: readonly ActivityEntry[]; empty?: ReactNode }) {
  if (entries.length === 0) {
    return <p className="kp-note">{empty ?? 'Nothing yet.'}</p>;
  }
  return (
    <ol className="kp-feed">
      {entries.map((entry) => (
        <li key={entry.id} className="kp-feed__item" data-state={entry.state}>
          <Glyph state={entry.state} />
          <div>
            <p className="kp-feed__text">
              {entry.lead && <b>{entry.lead} </b>}
              {entry.text}
            </p>
            {entry.detail && <p className="kp-feed__detail">{entry.detail}</p>}
            <span className="kp-feed__time">
              {entry.time}
              {entry.kind && ` · ${entry.kind}`}
            </span>
          </div>
        </li>
      ))}
    </ol>
  );
}

export interface FactsProps {
  title: string;
  facts: readonly { label: string; value: ReactNode; mono?: boolean }[];
}

/** A short block of facts, such as this machine's name, role, version and ports. */
export function Facts({ title, facts }: FactsProps) {
  return (
    <section className="kp-facts" aria-label={title}>
      <h2>{title}</h2>
      <dl className="kp-kv">
        {facts.map((fact) => (
          <div key={fact.label}>
            <dt>{fact.label}</dt>
            <dd className={fact.mono ? 'kp-type-mono-sm' : undefined}>{fact.value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}
