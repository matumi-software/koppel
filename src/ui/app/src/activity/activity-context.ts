import { createContext, useContext } from 'react';
import type { ActivityEvent as ActivityRow } from '@matumi/ui';
import type { ActivityEvent } from '@/api/activity';

/** Where a line stands: our own change still waiting, done, or refused. */
export type ActivityState = 'pending' | 'done' | 'fault';

/** One line in the log: from the server, or a change of ours still waiting or refused before it got there. */
export interface ActivityItem {
  id: string;
  state: ActivityState;
  text: string;
  detail?: string;
  at: Date;
  /** What the entry is about, such as a device's bus ID or a client ID, for that item's history. */
  subject?: string;
  kind?: string;
}

export interface ActivityApi {
  items: readonly ActivityItem[];
  /** Adds a local line, for a change sent but not yet confirmed. */
  add: (item: Omit<ActivityItem, 'id' | 'at'>) => string;
  update: (id: string, patch: Partial<Omit<ActivityItem, 'id'>>) => void;
  /** Drops a local line once the server has logged the outcome itself. */
  remove: (id: string) => void;
}

export const ActivityContext = createContext<ActivityApi | null>(null);

export function useActivity(): ActivityApi {
  const value = useContext(ActivityContext);
  if (!value) throw new Error('useActivity must be used inside ActivityProvider.');
  return value;
}

export function fromServer(event: ActivityEvent): ActivityItem {
  const failed = event.outcome === 'failed';
  return {
    id: event.id,
    state: failed ? 'fault' : 'done',
    text: event.summary,
    detail: failed ? (event.detail ?? undefined) : undefined,
    at: new Date(event.timestampUtc),
    subject: event.entity.id,
    kind: event.entity.type,
  };
}

const timeFormat = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });
const dayFormat = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short' });

const when = (at: Date) => (at.toDateString() === new Date().toDateString() ? timeFormat.format(at) : `${dayFormat.format(at)} ${timeFormat.format(at)}`);

/**
 * For Heartwood's ActivityPage and ActivityLog: newest first (the items already are), optionally
 * only about one subject. A refusal reads "Rejected" with the server's reason after the text, a
 * change still waiting reads "Pending".
 */
export function toEvents(items: readonly ActivityItem[], subject?: string): ActivityRow[] {
  return items
    .filter((item) => subject === undefined || item.subject === subject)
    .map((item) => ({
      id: item.id,
      time: when(item.at),
      text: item.detail ? `${item.text}: ${item.detail}` : item.text,
      result: item.state === 'fault' ? { label: 'Rejected' } : item.state === 'pending' ? { label: 'Pending', tone: 'info' as const } : undefined,
    }));
}

/** How many lines the log keeps, on the server and here. */
export const maxActivity = 200;

/**
 * The server's log with new events folded in by id, newest first. A fetch and a push can each hold
 * events the other lacks, so neither may replace the list.
 */
export function mergeActivity(current: readonly ActivityEvent[] | undefined, incoming: readonly ActivityEvent[]): ActivityEvent[] {
  const byId = new Map<string, ActivityEvent>();
  for (const event of current ?? []) byId.set(event.id, event);
  for (const event of incoming) byId.set(event.id, event);
  return [...byId.values()].sort((a, b) => Date.parse(b.timestampUtc) - Date.parse(a.timestampUtc)).slice(0, maxActivity);
}
