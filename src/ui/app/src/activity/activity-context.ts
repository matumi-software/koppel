import { createContext, useContext } from 'react';
import type { ActivityEntry, GlyphState } from '@koppel/react-ui';
import type { ServerActivity } from '@/api/activity';

/** One line in the log: from the server, or a change of ours still waiting or refused before it got there. */
export interface ActivityItem {
  id: string;
  state: GlyphState;
  lead?: string;
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

const glyphs: Record<ServerActivity['reason'], GlyphState> = {
  shared: 'shared',
  unshared: 'local',
  attached: 'attached',
  detached: 'local',
  plugged: 'live',
  unplugged: 'offline',
  clientAdded: 'shared',
  clientRevoked: 'local',
  secretRotated: 'live',
  reconfigured: 'live',
  linkUp: 'live',
  linkDown: 'fault',
  signInFailed: 'fault',
  passwordChanged: 'live',
  certificatePrepared: 'live',
  certificateCancelled: 'local',
  certificateSwitched: 'live',
  serverMoved: 'live',
};

export function fromServer(event: ServerActivity): ActivityItem {
  const failed = event.outcome === 'failed';
  return {
    id: event.id,
    state: failed ? 'fault' : glyphs[event.reason],
    lead: failed ? 'Not done.' : undefined,
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

/** For ActivityFeed: newest first, optionally only about one subject. */
export function toEntries(items: readonly ActivityItem[], subject?: string): ActivityEntry[] {
  return items
    .filter((item) => subject === undefined || item.subject === subject)
    .map((item) => ({ id: item.id, state: item.state, lead: item.lead, text: item.text, detail: item.detail, time: when(item.at), kind: subject ? undefined : item.kind }));
}

/** How many lines the log keeps, on the server and here. */
export const maxActivity = 200;

/**
 * The server's log with new events folded in by id, newest first. A fetch and a push can each hold
 * events the other lacks, so neither may replace the list.
 */
export function mergeActivity(current: readonly ServerActivity[] | undefined, incoming: readonly ServerActivity[]): ServerActivity[] {
  const byId = new Map<string, ServerActivity>();
  for (const event of current ?? []) byId.set(event.id, event);
  for (const event of incoming) byId.set(event.id, event);
  return [...byId.values()].sort((a, b) => Date.parse(b.timestampUtc) - Date.parse(a.timestampUtc)).slice(0, maxActivity);
}
