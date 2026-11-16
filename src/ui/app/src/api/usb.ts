import { getJson, sendChange } from './http';
import type { ClientServersView, ServerDevices } from './generated/wire';

export type { ClientDeviceView, ClientServersView, DeviceState, DeviceView, LinkState, ServerDevices, ServerDevicesView } from './generated/wire';

export const getServerDevices = () => getJson<ServerDevices>('/api/v1/server/devices');

export const setShared = (busId: string, shared: boolean) =>
  sendChange(`/api/v1/server/devices/${encodeURIComponent(busId)}/shared`, 'PUT', { shared });

export const getClientDevices = () => getJson<ClientServersView>('/api/v1/client/devices');

// A device is named by its server and its bus ID: two servers can offer the same bus ID.
const attachment = (serverId: string, busId: string) =>
  `/api/v1/client/servers/${encodeURIComponent(serverId)}/devices/${encodeURIComponent(busId)}/attachment`;

export const attachDevice = (serverId: string, busId: string) => sendChange(attachment(serverId, busId), 'PUT');

export const detachDevice = (serverId: string, busId: string) => sendChange(attachment(serverId, busId), 'DELETE');
