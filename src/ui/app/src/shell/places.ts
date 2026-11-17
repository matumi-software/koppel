import { Activity, CircleCheck, LayoutGrid, Link2, Monitor, Plug, Settings, Shield, Usb, type LucideIcon } from 'lucide-react';
import type { NavGroup, NavItem } from '@heartwood/ui';
import { isClientRole, isServerRole } from '@/settings/roles';

/**
 * The app's places. The id is what the address holds (`#remote`), so a phone can bookmark a page.
 * `Page` is also what `goTo` takes.
 */
export const places = {
  overview: { label: 'Overview', icon: LayoutGrid },
  devices: { label: 'Devices', icon: Usb },
  remote: { label: 'Remote devices', shortLabel: 'Remote', icon: Plug },
  clients: { label: 'Clients', icon: Monitor },
  activity: { label: 'Activity', icon: Activity },
  check: { label: 'System check', shortLabel: 'Check', icon: CircleCheck },
  connection: { label: 'Connection', icon: Link2 },
  security: { label: 'Security', icon: Shield },
  settings: { label: 'Settings', icon: Settings },
} satisfies Record<string, { label: string; shortLabel?: string; icon: LucideIcon }>;

export type Page = keyof typeof places;

export const isPage = (value: string): value is Page => Object.hasOwn(places, value);

const item = (page: Page, count?: number): NavItem => ({ id: page, ...places[page], count });

/** What the nav counts: only what asks for action, never totals (Heartwood). */
export interface NavCounts {
  overview?: number;
}

/**
 * The navigation for a role, grouped as the prototype groups it: the everyday places, Manage with
 * Clients (a server) and System check, and Configure. A group of one folds into the first group.
 * Until the role is known (`undefined`) the machine is neither, so only the places every role has show.
 */
export function navGroups(role: string | undefined, counts: NavCounts = {}): NavGroup[] {
  const server = isServerRole(role);
  const client = isClientRole(role);
  const first = [item('overview', counts.overview), server && item('devices'), client && item('remote'), item('activity')].filter(Boolean) as NavItem[];
  const manage = [server && item('clients'), item('check')].filter(Boolean) as NavItem[];
  if (manage.length === 1) first.push(manage.pop()!);
  const configure = [item('connection'), item('security'), item('settings')];
  return [{ items: first }, ...(manage.length ? [{ label: 'Manage', items: manage }] : []), { label: 'Configure', items: configure }];
}

/** The pages a role has, in nav order. A bookmarked page the role lacks falls back to the overview. */
export const pagesOf = (role: string | undefined): Page[] => navGroups(role).flatMap((group) => group.items.map((i) => i.id as Page));
