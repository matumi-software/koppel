import type { RadioOption } from '@heartwood/ui';
import type { Role } from '@/api/settings';

/** The three roles, worded the same in the first run and in Settings. */
export const roleOptions: readonly (RadioOption & { value: Role })[] = [
  { value: 'server', label: 'Share its devices (server)' },
  { value: 'client', label: 'Attach devices from a server (client)' },
  { value: 'both', label: 'Both' },
];

/**
 * What a role does. A role this UI does not know (a newer server) does neither, so nothing
 * is shown or enabled on a guess.
 */
export const isServerRole = (role: string | undefined) => role === 'server' || role === 'both';
export const isClientRole = (role: string | undefined) => role === 'client' || role === 'both';

/** The role as shown to a person. */
export const roleName = (role: string | undefined) => (role === 'server' ? 'Server' : role === 'client' ? 'Client' : role === 'both' ? 'Server and client' : 'Unknown');
