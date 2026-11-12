import { getJson, sendChange } from './http';
import type { ClientDevices, ServerDevices } from './generated/wire';

export type { ClientDeviceView, ClientDevices, DeviceState, DeviceView, LinkState, ServerDevices } from './generated/wire';

export const getServerDevices = () => getJson<ServerDevices>('/api/server/devices');

export const setShared = (busId: string, shared: boolean) =>
  sendChange(`/api/server/devices/${encodeURIComponent(busId)}/shared`, 'PUT', { shared });

export const getClientDevices = () => getJson<ClientDevices>('/api/client/devices');

export const attachDevice = (busId: string) => sendChange(`/api/client/devices/${encodeURIComponent(busId)}/attach`, 'POST');

export const detachDevice = (busId: string) => sendChange(`/api/client/devices/${encodeURIComponent(busId)}/detach`, 'POST');
