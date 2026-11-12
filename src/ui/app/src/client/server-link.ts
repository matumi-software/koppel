import type { LinkStatus } from '@/api/usb';

/** A version as it reads on screen: v0.1.1 for a number, a build name such as dev as it is. */
export const formatVersion = (version: string | undefined) => (version === undefined ? undefined : /^\d/.test(version) ? `v${version}` : version);

/** What a client's link to its server says about itself, for the status card. Pending until it is connected. */
export function serverLine(link: LinkStatus, name: string | null | undefined): { text: string; tone?: 'pending' } {
  switch (link) {
    case 'connected':
      return { text: `Linked to ${name ?? 'the server'}.` };
    case 'unconfigured':
      return { text: 'No server connection yet.', tone: 'pending' };
    case 'connecting':
      return { text: `Connecting to ${name ?? 'the server'}…`, tone: 'pending' };
    default:
      return { text: `Cannot reach ${name ?? 'the server'}.`, tone: 'pending' };
  }
}
