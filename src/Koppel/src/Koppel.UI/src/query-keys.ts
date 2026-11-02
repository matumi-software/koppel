/** Every query key the app uses, so a hub push and the page that reads it name the same cache entry. */
export const settingsKey = ['settings'] as const;
export const clientsKey = ['clients'] as const;
export const connectionKey = ['connection'] as const;
export const serverDevicesKey = ['server-devices'] as const;
export const clientDevicesKey = ['client-devices'] as const;
export const activityKey = ['activity'] as const;
export const pairingRequestsKey = ['pairing-requests'] as const;
export const pairingStatusKey = ['pairing-status'] as const;
export const discoveredServersKey = ['discovered-servers'] as const;
export const systemInfoKey = ['system-info'] as const;
export const authKey = ['auth-state'] as const;
