import type { ReactNode } from 'react';
import { Button } from './Button';
import { Glyph, type GlyphState } from './Activity';
import { SectionHeading } from './Text';

export interface DrawerProps {
  title: string;
  /** Above the title, such as a StatusBadge. */
  badge?: ReactNode;
  /** Under the title, such as the maker or when it was added. */
  subtitle?: ReactNode;
  /** Close returns the aside to the activity feed. */
  onClose: () => void;
  children: ReactNode;
}

/**
 * The selected item's details in the aside: a title with Close, then its parts. The same shape for
 * a device and for a client, so the aside reads the same whatever is open.
 */
export function Drawer({ title, badge, subtitle, onClose, children }: DrawerProps) {
  return (
    <>
      <div className="kp-drawer__head">
        <div className="kp-drawer__title">
          {badge}
          <h2>{title}</h2>
          {subtitle}
        </div>
        <Button variant="ghost" aria-label="Close details" onPress={onClose}>
          Close
        </Button>
      </div>
      {children}
    </>
  );
}

/** A part of a drawer set off by a rule, such as its History. */
export function DrawerSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="kp-drawer__section">
      <SectionHeading level={3}>{title}</SectionHeading>
      {children}
    </div>
  );
}

export interface HolderProps {
  state: GlyphState;
  title: string;
  detail: string;
}

/** Who has the device and what that means, in the colour of its state. */
export function Holder({ state, title, detail }: HolderProps) {
  return (
    <div className="kp-holder" data-state={state}>
      <Glyph state={state} />
      <div className="kp-holder__text">
        <span className="kp-type-label">{title}</span>
        <span className="kp-type-caption">{detail}</span>
      </div>
    </div>
  );
}

export interface KeyValueProps {
  items: readonly { label: string; value: ReactNode; mono?: boolean }[];
}

/** Facts as a list of labels and values, such as a device's VID:PID, bus and speed. */
export function KeyValues({ items }: KeyValueProps) {
  return (
    <dl className="kp-kv">
      {items.map((item) => (
        <div key={item.label}>
          <dt>{item.label}</dt>
          <dd className={item.mono ? 'kp-type-mono' : undefined}>{item.value}</dd>
        </div>
      ))}
    </dl>
  );
}

/** Short names side by side, such as a device's interfaces or the devices a client has attached. */
export function Chips({ items }: { items: readonly string[] }) {
  return (
    <span className="kp-chips">
      {items.map((name) => (
        <span key={name} className="kp-chip">
          {name}
        </span>
      ))}
    </span>
  );
}
