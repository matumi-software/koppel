import type { RoleOption } from '@koppel/react-ui';
import type { KoppelRole } from '@/api/settings';

/** The three roles, worded the same in the first run and under Configuration. */
export const roleOptions: readonly RoleOption<KoppelRole>[] = [
  { value: 'server', title: 'Server', description: 'Shares the USB devices plugged into this machine with other machines on the network.' },
  { value: 'client', title: 'Client', description: 'Attaches devices from a Koppel server, so programs here see them as if they were plugged in.' },
  { value: 'both', title: 'Both', description: 'Shares its own devices and attaches devices from another server.' },
];

/**
 * What a role does. A role this UI does not know (a newer server) does neither, so nothing
 * is shown or enabled on a guess.
 */
export const isServerRole = (role: string | undefined) => role === 'server' || role === 'both';
export const isClientRole = (role: string | undefined) => role === 'client' || role === 'both';

/** The role as shown to a person. */
export const roleName = (role: string | undefined) => roleOptions.find((o) => o.value === role)?.title ?? 'Unknown';
