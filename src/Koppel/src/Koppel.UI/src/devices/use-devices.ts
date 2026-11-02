import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { getClientDevices, getServerDevices, type ClientDevices, type ServerDevices } from '@/api/usb';
import { useHubEvent } from '@/hub/hub-context';

export const serverDevicesKey = ['server-devices'] as const;
export const clientDevicesKey = ['client-devices'] as const;

/** This machine's devices (server role), kept current by DevicesChanged. */
export function useServerDevices(enabled = true, onPush?: (devices: ServerDevices) => void) {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: serverDevicesKey, queryFn: getServerDevices, enabled });
  const onChanged = useCallback(
    (next: ServerDevices) => {
      queryClient.setQueryData(serverDevicesKey, next);
      onPush?.(next);
    },
    [queryClient, onPush],
  );
  useHubEvent('DevicesChanged', onChanged);
  return query;
}

/** The server's shared devices and what is attached here (client role), kept current by ClientDevicesChanged. */
export function useClientDevices(enabled = true, onPush?: (devices: ClientDevices) => void) {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: clientDevicesKey, queryFn: getClientDevices, enabled });
  const onChanged = useCallback(
    (next: ClientDevices) => {
      queryClient.setQueryData(clientDevicesKey, next);
      onPush?.(next);
    },
    [queryClient, onPush],
  );
  useHubEvent('ClientDevicesChanged', onChanged);
  return query;
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
