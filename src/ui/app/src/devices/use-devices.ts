import { useQuery } from '@tanstack/react-query';
import { clientDevicesOptions, serverDevicesOptions } from '@/queries';
import { clock } from '@/shared/time';

/** This machine's devices (server role). DevicesChanged keeps them current, see live/push-table.ts. */
export function useServerDevices(enabled = true) {
  return useQuery({ ...serverDevicesOptions(), enabled });
}

/**
 * Every server this machine attaches from, each with its link and its devices (client role).
 * ClientDevicesChanged keeps them current. Read `data.servers[i].devices`.
 */
export function useClientDevices(enabled = true) {
  return useQuery({ ...clientDevicesOptions(), enabled });
}


/** "since 09:14", or nothing when unknown. */
export const since = (iso: string | null) => (iso ? ` since ${clock.format(new Date(iso))}` : '');

/** The server's host name from its address, for labels such as "From lab-rack-3". */
export const hostOf = (url: string | null | undefined) => {
  try {
    return url ? new URL(url).hostname : null;
  } catch {
    return null;
  }
};
