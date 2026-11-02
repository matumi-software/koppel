import type { Key, ReactNode } from 'react';
import { Button, Disclosure, DisclosurePanel, GridList, GridListItem, Heading } from 'react-aria-components';
import type { DeviceState } from './StatusBadge';

export interface DeviceGroupProps {
  title: string;
  count: number;
  defaultExpanded?: boolean;
  children: ReactNode;
}

/** Devices that share a state, such as "Waiting for the server". A react-aria Disclosure. */
export function DeviceGroup({ title, count, defaultExpanded = true, children }: DeviceGroupProps) {
  return (
    <Disclosure className="kp-group" defaultExpanded={defaultExpanded}>
      <Heading level={2} style={{ margin: 0 }}>
        <Button slot="trigger" className="kp-group__head">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <path d="m9 18 6-6-6-6" />
          </svg>
          {title}
          <span className="kp-group__count">{count}</span>
        </Button>
      </Heading>
      <DisclosurePanel>{children}</DisclosurePanel>
    </Disclosure>
  );
}

export interface DeviceCardsProps<T> {
  label: string;
  items: readonly T[];
  getKey: (item: T) => string;
  getName: (item: T) => string;
  getState: (item: T) => DeviceState;
  /** The card open in the detail drawer, if any. */
  selectedKey?: string | null;
  onSelectionChange?: (key: string | null) => void;
  /** The card's content; DeviceCardBody fits here. */
  children: (item: T) => ReactNode;
}

/**
 * Device cards as a react-aria GridList: arrow keys move between cards, Enter or a click selects one
 * for the detail drawer, and switches and buttons inside a card still work on their own.
 */
export function DeviceCards<T>({ label, items, getKey, getName, getState, selectedKey, onSelectionChange, children }: DeviceCardsProps<T>) {
  return (
    <GridList
      aria-label={label}
      className="kp-cards"
      layout="grid"
      selectionMode={onSelectionChange ? 'single' : 'none'}
      selectionBehavior="replace"
      selectedKeys={selectedKey ? [selectedKey] : []}
      onSelectionChange={(keys) => {
        const [first] = keys === 'all' ? [] : [...keys];
        onSelectionChange?.((first as Key | undefined)?.toString() ?? null);
      }}
    >
      {items.map((item) => (
        <GridListItem key={getKey(item)} id={getKey(item)} textValue={getName(item)} className="kp-device" data-state={getState(item)}>
          {children(item)}
        </GridListItem>
      ))}
    </GridList>
  );
}
