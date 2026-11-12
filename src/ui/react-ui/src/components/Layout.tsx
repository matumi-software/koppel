import type { ElementType, ReactNode } from 'react';

type Gap = 'sm' | 'md' | 'lg';

export interface CardProps {
  /** Names the card for screen readers: it is a labelled region. */
  label: string;
  /** Space between the card's children. The default is the design system's 16px. */
  gap?: 'sm' | 'md';
  /** No padding and no gap, for a card that holds a list of rows that draw their own. */
  isFlush?: boolean;
  children: ReactNode;
}

/** A plain container on the page ground: forms, lists and notes that are not a Panel. */
export function Card({ label, gap, isFlush, children }: CardProps) {
  return (
    <section className="kp-card" aria-label={label} data-gap={gap} data-flush={isFlush ? 'true' : undefined}>
      {children}
    </section>
  );
}

export interface StackProps {
  gap?: Gap;
  /** Keep children at their own width instead of stretching them. */
  align?: 'start';
  children: ReactNode;
}

/** Children one under the other with the design system's spacing. */
export function Stack({ gap, align, children }: StackProps) {
  return (
    <div className="kp-stack" data-gap={gap} data-align={align}>
      {children}
    </div>
  );
}

export interface RowProps {
  gap?: 'xs';
  justify?: 'end' | 'between';
  /** The element to render, such as 'span' inside a line of text. */
  as?: ElementType;
  children: ReactNode;
}

/** Children side by side, wrapping when the room runs out. */
export function Row({ gap, justify, as: Tag = 'div', children }: RowProps) {
  return (
    <Tag className="kp-row" data-gap={gap} data-justify={justify}>
      {children}
    </Tag>
  );
}
