import type { ReactNode } from 'react';

export interface PanelProps {
  title: string;
  /** Shown at the right of the header, such as "4 of 6". */
  meta?: ReactNode;
  /** "needs" draws the amber Needs you panel. */
  tone?: 'needs';
  footer?: ReactNode;
  children: ReactNode;
}

/** A titled box on the overview, usually holding a MiniList. */
export function Panel({ title, meta, tone, footer, children }: PanelProps) {
  return (
    <section className="kp-panel" data-tone={tone} aria-label={title}>
      <div className="kp-panel__head">
        <h2>{title}</h2>
        {meta && <span className="kp-type-caption">{meta}</span>}
      </div>
      {children}
      {footer && <div className="kp-panel__foot">{footer}</div>}
    </section>
  );
}

export interface MiniListItem {
  id: string;
  name: ReactNode;
  /** Badge or button at the right. */
  end?: ReactNode;
  /** The line under the name. */
  sub?: ReactNode;
}

export function MiniList({ items, empty }: { items: readonly MiniListItem[]; empty: string }) {
  if (items.length === 0) {
    return <p className="kp-mini__empty">{empty}</p>;
  }
  return (
    <ul className="kp-mini">
      {items.map((item) => (
        <li key={item.id}>
          <span className="kp-mini__name">{item.name}</span>
          {item.end ?? <span />}
          {item.sub && <span className="kp-mini__sub">{item.sub}</span>}
        </li>
      ))}
    </ul>
  );
}

export interface SettingProps {
  title: string;
  /** What the setting does, under the title. */
  description?: ReactNode;
  /** The control: a Switch or Field with its label hidden, since the title names it. */
  control: ReactNode;
}

/** Settings rows inside a Panel: the text on the left, the control on the right. */
export function SettingList({ label, children }: { label: string; children: ReactNode }) {
  return (
    <ul className="kp-settings" aria-label={label}>
      {children}
    </ul>
  );
}

export function Setting({ title, description, control }: SettingProps) {
  return (
    <li className="kp-setting">
      <div className="kp-setting__text">
        <span className="kp-setting__title">{title}</span>
        {description && <span className="kp-setting__help">{description}</span>}
      </div>
      <div className="kp-setting__control">{control}</div>
    </li>
  );
}
