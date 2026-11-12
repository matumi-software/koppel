import type { ReactNode } from 'react';

export interface NoteProps {
  /** `error` for what went wrong and `ok` for what went right; plain is supporting text. */
  tone?: 'error' | 'ok';
  /** Space around it, for a note that stands alone on the page, such as "Loading". */
  isPadded?: boolean;
  children: ReactNode;
}

/** A line of text under or between controls. It is not announced: see the Announcer for that. */
export function Note({ tone, isPadded, children }: NoteProps) {
  const className = tone === 'error' ? 'kp-error' : tone === 'ok' ? 'kp-ok' : 'kp-note';
  return (
    <p className={className} data-padded={isPadded ? 'true' : undefined}>
      {children}
    </p>
  );
}

export interface CaptionProps {
  /** Dimmed, or in the pending or fault colour of the state it describes. */
  tone?: 'faint' | 'pending' | 'fault';
  children: ReactNode;
}

/** Small text beside or under a name: when it was added, what it is doing. */
export function Caption({ tone, children }: CaptionProps) {
  return (
    <span className="kp-type-caption" data-tone={tone}>
      {children}
    </span>
  );
}

/** A heading inside a page or a panel, in the heading type size and without the browser's margins. */
export function SectionHeading({ level = 2, children }: { level?: 2 | 3; children: ReactNode }) {
  const Tag = level === 2 ? 'h2' : 'h3';
  return <Tag className="kp-type-heading kp-heading">{children}</Tag>;
}

/** A bulleted list of supporting text, such as the ways to restart Koppel. */
export function NoteList({ children }: { children: ReactNode }) {
  return <ul className="kp-note">{children}</ul>;
}

/** An identifier as the hardware or the server spells it: a client ID, a key ID, an address. */
export function Identifier({ children }: { children: ReactNode }) {
  return <code className="kp-id">{children}</code>;
}

/** Small monospaced text for numbers that line up, such as VID:PID. */
export function Mono({ children }: { children: ReactNode }) {
  return <span className="kp-type-mono-sm">{children}</span>;
}
