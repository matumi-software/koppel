import type { ReactNode, Ref } from 'react';
import { KoppelMark } from './KoppelMark';

export interface FirstRunShellProps {
  /** The page's own name, such as "Sign in". It becomes the document title with the product name. */
  title: string;
  /** `wide` for the first run's role step; the default fits a form. */
  width?: 'wide';
  children: ReactNode;
}

/** The plain page that sign-in and first run sit on: the Koppel mark, then one column. */
export function FirstRunShell({ title, width, children }: FirstRunShellProps) {
  return (
    <div className="kp-firstrun" data-width={width}>
      <title>{`${title} - Koppel`}</title>
      <header className="kp-firstrun__bar">
        <KoppelMark size={26} />
        <span>koppel</span>
      </header>
      <main className="kp-firstrun__main">
        <div className="kp-firstrun__column">{children}</div>
      </main>
    </div>
  );
}

export interface PageHeadProps {
  title: string;
  lede?: ReactNode;
  /** For the heading to take focus when the page opens, so a screen reader starts at the new page. */
  headingRef?: Ref<HTMLHeadingElement>;
}

/** The page's heading and the sentence under it. A page has one, and it is the page's h1. */
export function PageHead({ title, lede, headingRef }: PageHeadProps) {
  return (
    <div className="kp-page-head">
      <h1 ref={headingRef} tabIndex={headingRef ? -1 : undefined}>
        {title}
      </h1>
      {lede && <p>{lede}</p>}
    </div>
  );
}
