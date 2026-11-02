import { createContext, useContext } from 'react';
import type { ActivityEntry, GlyphState } from '@koppel/react-ui';

export interface ActivityItem {
  id: string;
  state: GlyphState;
  lead?: string;
  text: string;
  at: Date;
  /** What the entry is about, such as a device's bus ID, for that device's history. */
  subject?: string;
}

export interface ActivityApi {
  items: readonly ActivityItem[];
  add: (item: Omit<ActivityItem, 'id' | 'at'>) => string;
  update: (id: string, patch: Partial<Omit<ActivityItem, 'id'>>) => void;
}

export const ActivityContext = createContext<ActivityApi | null>(null);

export function useActivity(): ActivityApi {
  const value = useContext(ActivityContext);
  if (!value) throw new Error('useActivity must be used inside ActivityProvider.');
  return value;
}

const timeFormat = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });

/** For ActivityFeed: newest first, optionally only about one subject. */
export function toEntries(items: readonly ActivityItem[], subject?: string): ActivityEntry[] {
  return items
    .filter((item) => subject === undefined || item.subject === subject)
    .map((item) => ({ id: item.id, state: item.state, lead: item.lead, text: item.text, time: timeFormat.format(item.at) }));
}
