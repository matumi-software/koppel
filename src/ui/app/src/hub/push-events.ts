/** Where the server pushes, as Server-Sent Events. */
export const eventsUrl = '/api/events';

/** Every event the server pushes, each carrying the value as written to disk. */
export const pushEvents = [
  'SettingsChanged',
  'ClientsChanged',
  'ServerKeysChanged',
  'ConnectionChanged',
  'DevicesChanged',
  'ClientDevicesChanged',
  'ActivityRecorded',
  'ServersDiscovered',
  'PairingStatusChanged',
  'PairingRequestsChanged',
] as const;

const retryDelays = [0, 2000, 5000, 10000, 30000];

/** How long to wait before reconnect number `attempt` (from 0). It keeps retrying; the last delay repeats. */
export const retryDelay = (attempt: number) => retryDelays[Math.min(attempt, retryDelays.length - 1)];
