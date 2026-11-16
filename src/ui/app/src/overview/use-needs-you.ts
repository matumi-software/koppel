import { useQuery } from '@tanstack/react-query';
import { CircleCheck, Link2, RefreshCw, UserPlus } from 'lucide-react';
import type { NeedsYouItem } from '@heartwood/ui';
import { useClientDevices, useServerDevices } from '@/devices/use-devices';
import { remoteEntries } from '@/devices/device-words';
import { usePending } from '@/pending/pending-context';
import { pairingRequestsOptions, systemChecksOptions } from '@/queries';
import { isClientRole, isServerRole } from '@/settings/roles';
import { useSettings } from '@/settings/use-settings';
import { goTo } from '@/shell/frame-context';
import { checksForRole } from '@/system/for-role';

/** What asks for action, in the order Overview lists it. The nav badge counts this same list. */
export function useNeedsYou(locked: boolean): NeedsYouItem[] {
  const role = useSettings().query.data?.role;
  const server = isServerRole(role);
  const client = isClientRole(role);
  const devices = useServerDevices(server).data?.devices ?? [];
  const servers = useClientDevices(client).data?.servers ?? [];
  const checks = useQuery(systemChecksOptions()).data;
  const pairing = useQuery({ ...pairingRequestsOptions(), enabled: server }).data;
  const sharing = usePending('serverDevices');
  const attaching = usePending('clientDevices');
  const remote = remoteEntries(servers);

  return [
    ...devices
      .filter((d) => d.fault)
      .map((d) => ({
        id: `fault-${d.busId}`,
        title: `${d.name} is not shared`,
        description: d.fault ?? undefined,
        action: { label: `Share ${d.name} again`, icon: RefreshCw, iconOnly: true, disabled: locked, pending: d.busId in sharing.pending, onPress: () => sharing.request({ busId: d.busId, share: true }) },
      })),
    ...remote
      .filter((r) => r.entry.fault)
      .map((r) => ({
        id: `attach-${r.key}`,
        title: `${r.entry.device.name} did not attach`,
        description: r.entry.fault ?? undefined,
        action: { label: `Attach ${r.entry.device.name} again`, icon: RefreshCw, iconOnly: true, disabled: locked, pending: r.key in attaching.pending, onPress: () => attaching.request({ serverId: r.server.id, busId: r.entry.device.busId, attach: true }) },
      })),
    ...servers
      .filter((s) => s.link === 'error')
      .map((s) => ({
        id: `link-${s.id}`,
        title: `Cannot reach ${s.name}`,
        description: 'Devices attached from it stop working until it is back.',
        action: { label: 'Open Connection', icon: Link2, onPress: () => goTo('connection') },
      })),
    ...checksForRole(checks ?? [], server, client)
      .filter((c) => !c.ok)
      .map((c) => ({ id: `check-${c.id}`, title: c.title.replace(/\.$/, ''), description: c.detail, action: { label: 'System check', icon: CircleCheck, onPress: () => goTo('check') } })),
    ...(pairing?.requests ?? [])
      .filter((r) => r.state === 'waiting')
      .map((r) => ({ id: `pair-${r.id}`, title: `${r.clientName} wants to pair`, description: `From ${r.from}. Type the code it shows on Clients.`, action: { label: 'Review', icon: UserPlus, onPress: () => goTo('clients') } })),
  ];
}
