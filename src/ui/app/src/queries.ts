import { queryOptions } from '@tanstack/react-query';
import { mergeActivity } from '@/activity/activity-context';
import { getActivity } from '@/api/activity';
import { getAuthState } from '@/api/auth';
import { getConnection } from '@/api/client';
import { getClients, getIdentity, getServerKeys } from '@/api/server';
import { getSettings } from '@/api/settings';
import { getDiscoveredServers, getPairingRequests, getPairingStatus } from '@/api/pairing';
import { getSystemChecks, getSystemInfo } from '@/api/system';
import { getClientDevices, getServerDevices } from '@/api/usb';

/**
 * Every query the app runs: its key and how it is fetched, one factory per resource. Hooks, the live
 * push (live/push-table.ts), the pending domains and the tests all take the key from here, so they
 * name the same cache entry. Spread the options to add what a caller needs (`enabled`, an interval).
 */
export const settingsOptions = () => queryOptions({ queryKey: ['settings'], queryFn: getSettings });
export const clientsOptions = () => queryOptions({ queryKey: ['clients'], queryFn: getClients });
export const identityOptions = () => queryOptions({ queryKey: ['identity'], queryFn: getIdentity });
export const serverKeysOptions = () => queryOptions({ queryKey: ['server-keys'], queryFn: getServerKeys });
export const connectionOptions = () => queryOptions({ queryKey: ['connection'], queryFn: getConnection });
export const serverDevicesOptions = () => queryOptions({ queryKey: ['server-devices'], queryFn: getServerDevices });
export const clientDevicesOptions = () => queryOptions({ queryKey: ['client-devices'], queryFn: getClientDevices });
export const pairingRequestsOptions = () => queryOptions({ queryKey: ['pairing-requests'], queryFn: getPairingRequests });
export const pairingStatusOptions = () => queryOptions({ queryKey: ['pairing-status'], queryFn: getPairingStatus });
export const discoveredServersOptions = () => queryOptions({ queryKey: ['discovered-servers'], queryFn: getDiscoveredServers });
export const systemInfoOptions = () => queryOptions({ queryKey: ['system-info'], queryFn: getSystemInfo });
export const systemChecksOptions = () => queryOptions({ queryKey: ['system-checks'], queryFn: getSystemChecks });
export const authOptions = () => queryOptions({ queryKey: ['auth-state'], queryFn: getAuthState });

/** Events pushed while the fetch ran must survive it, so the cache is read after the await. */
const activityKey = ['activity'] as const;
export const activityOptions = () =>
  queryOptions({
    queryKey: activityKey,
    queryFn: async ({ client }) => {
      const fetched = await getActivity();
      return mergeActivity(client.getQueryData(activityKey), fetched);
    },
  });
