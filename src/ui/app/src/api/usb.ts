import { getJson, sendChange } from './http';
import type { ClientDevices, ServerDevices } from './generated/wire';

export type { ClientDeviceView, ClientDevices, DeviceState, DeviceView, LinkState, ServerDevices } from './generated/wire';

export const getServerDevices = () => getJson<ServerDevices>('/api/v1/server/devices');

export const setShared = (busId: string, shared: boolean) =>
  sendChange(`/api/v1/server/devices/${encodeURIComponent(busId)}/shared`, 'PUT', { shared });

export const getClientDevices = () => getJson<ClientDevices>('/api/v1/client/devices');

export const attachDevice = (busId: string) => sendChange(`/api/v1/client/devices/${encodeURIComponent(busId)}/attachment`, 'PUT');

export const detachDevice = (busId: string) => sendChange(`/api/v1/client/devices/${encodeURIComponent(busId)}/attachment`, 'DELETE');
