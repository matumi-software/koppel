import type { Key, ReactNode } from 'react';
import { GridList, GridListItem } from 'react-aria-components';

export interface ItemListProps<T> {
  label: string;
  items: readonly T[];
  getKey: (item: T) => string;
  getText: (item: T) => string;
  /** The row open in the detail drawer, if any. */
  selectedKey?: string | null;
  onSelectionChange: (key: string | null) => void;
  /** Marks a row as waiting for the server. */
  isPending?: (item: T) => boolean;
  children: (item: T) => ReactNode;
}

/**
 * Rows you can open, such as registered clients. A react-aria GridList: arrow keys move, Enter or a
 * click selects, and buttons inside a row still work on their own.
 */
export function ItemList<T>({ label, items, getKey, getText, selectedKey, onSelectionChange, isPending, children }: ItemListProps<T>) {
  return (
    <GridList
      aria-label={label}
      className="kp-list"
      selectionMode="single"
      selectionBehavior="replace"
      selectedKeys={selectedKey ? [selectedKey] : []}
      onSelectionChange={(keys) => {
        const [first] = keys === 'all' ? [] : [...keys];
        onSelectionChange((first as Key | undefined)?.toString() ?? null);
      }}
    >
      {items.map((item) => (
        <GridListItem
          key={getKey(item)}
          id={getKey(item)}
          textValue={getText(item)}
          className="kp-list__item"
          data-state={isPending?.(item) ? 'pending' : undefined}
        >
          {children(item)}
        </GridListItem>
      ))}
    </GridList>
  );
}
