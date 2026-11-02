import type { DeviceState } from '@koppel/react-ui';
import { getJson, sendChange } from './http';

/** Mirrors Koppel.Usb.DeviceView. */
export interface DeviceView {
  busId: string;
  vidPid: string;
  name: string;
  manufacturer: string | null;
  speed: string;
  state: Exclude<DeviceState, 'pending'>;
  attachedTo: string | null;
  attachedSince: string | null;
  fault: string | null;
  serial: string | null;
  driver: string | null;
  interfaces: string[] | null;
}

export interface ServerDevices {
  devices: DeviceView[];
  unavailable: string | null;
}

export interface ClientDeviceView {
  device: DeviceView;
  attachedHere: boolean;
  /** Null when attached through the gateway: the other machine's driver holds it, not this kernel. */
  port: number | null;
  attachedSince: string | null;
  fault: string | null;
  /** Where a gateway attach came from, such as 172.17.0.1. */
  attachedVia: string | null;
}

export type LinkStatus = 'unconfigured' | 'connecting' | 'connected' | 'error';

export interface ClientDevices {
  link: LinkStatus;
  linkMessage: string | null;
  devices: ClientDeviceView[];
  unavailable: string | null;
  /** True while the plain USB/IP gateway listens, for a driver such as usbip-win2 on the host. */
  gateway: boolean;
}

export const getServerDevices = () => getJson<ServerDevices>('/api/server/devices');

export const setShared = (busId: string, shared: boolean) =>
  sendChange(`/api/server/devices/${encodeURIComponent(busId)}/shared`, 'PUT', { shared });

export const getClientDevices = () => getJson<ClientDevices>('/api/client/devices');

export const attachDevice = (busId: string) => sendChange(`/api/client/devices/${encodeURIComponent(busId)}/attach`, 'POST');

export const detachDevice = (busId: string) => sendChange(`/api/client/devices/${encodeURIComponent(busId)}/detach`, 'POST');
