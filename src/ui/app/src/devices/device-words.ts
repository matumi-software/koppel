import type { ClientDeviceView, DeviceView, ServerDevicesView } from '@/api/usb';
import { since } from './use-devices';

/** The word for one of this machine's devices, from Koppel's fixed state words (brand.ts). */
export const deviceWord = (d: DeviceView) => (d.state === 'attached' ? 'Attached' : d.state === 'shared' ? 'Shared' : d.state === 'offline' ? 'Offline' : 'Not shared');

/** The word for a device of a server, as seen from this machine. */
/** `link` is the server's: a device cannot be attached or used while its server cannot be reached. */
export const remoteWord = (e: ClientDeviceView, link?: string) =>
  link === 'error' ? 'Offline' : e.attachedHere ? 'Attached' : e.device.state === 'offline' ? 'Offline' : e.device.state === 'attached' ? 'In use' : 'Not attached';

/** What a person can read on a card's second line, for a device of this machine. */
export const deviceSub = (d: DeviceView, holder: string | null, fault: string | null | undefined) => {
  // The tile's status already says Not shared; this line says why.
  if (fault) return fault;
  if (d.state === 'attached') return `By ${holder ?? 'a client'}${since(d.attachedSince)}`;
  if (d.state === 'offline') return 'Unplugged';
  return `${d.manufacturer ?? 'Unknown maker'}, bus ${d.busId}`;
};

/** The same for a device of a server. */
export const remoteSub = (e: ClientDeviceView, server: string, fault: string | null | undefined, link?: string) => {
  if (fault) return fault;
  if (link === 'error') return `Cannot reach ${server}`;
  if (e.attachedHere) return `From ${server}${since(e.attachedSince)}`;
  if (e.device.state === 'offline') return `Unplugged on ${server}`;
  if (e.device.state === 'attached') return `Another client of ${server} has it`;
  return `${e.device.manufacturer ?? 'Unknown maker'} on ${server}`;
};

/** A device's kind: the interfaces it presents, joined. */
export const kindOf = (d: DeviceView) => (d.interfaces && d.interfaces.length > 0 ? d.interfaces.join(', ') : null);

/** What the remote devices of every server look like together: each entry knows its server. */
export interface RemoteEntry {
  /** The pending key and the history subject: server id and bus ID. */
  key: string;
  server: ServerDevicesView;
  entry: ClientDeviceView;
  /** The usbip command is shown only where no earlier server also offers this bus ID. */
  unambiguous: boolean;
}

export function remoteEntries(servers: readonly ServerDevicesView[]): RemoteEntry[] {
  const earlier = new Set<string>();
  const out: RemoteEntry[] = [];
  for (const server of servers) {
    const here = new Set<string>();
    for (const entry of server.devices) {
      const bus = entry.device.busId;
      out.push({ key: `${server.id}/${bus}`, server, entry, unambiguous: !earlier.has(bus) });
      here.add(bus);
    }
    here.forEach((bus) => earlier.add(bus));
  }
  return out;
}

/** "On lab", "On lab and bench", "On 3 servers". */
export const serversTitle = (servers: readonly { name: string }[]) =>
  servers.length === 0 ? 'Remote devices' : servers.length === 1 ? `On ${servers[0].name}` : servers.length === 2 ? `On ${servers[0].name} and ${servers[1].name}` : `On ${servers.length} servers`;
