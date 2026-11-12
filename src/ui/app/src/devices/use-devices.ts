import { useQuery } from '@tanstack/react-query';
import { getClientDevices, getServerDevices } from '@/api/usb';
import { clientDevicesKey, serverDevicesKey } from '@/query-keys';

/** This machine's devices (server role). DevicesChanged keeps them current, see live/push-table.ts. */
export function useServerDevices(enabled = true) {
  return useQuery({ queryKey: serverDevicesKey, queryFn: getServerDevices, enabled });
}

/** The server's shared devices and what is attached here (client role). ClientDevicesChanged keeps them current. */
export function useClientDevices(enabled = true) {
  return useQuery({ queryKey: clientDevicesKey, queryFn: getClientDevices, enabled });
}

const clock = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });

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
