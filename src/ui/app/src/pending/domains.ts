import type { ConnectionView } from '@/api/client';
import type { ClientView, ServerKeysView } from '@/api/server';
import type { KoppelRole, KoppelSettings } from '@/api/settings';
import type { DiscoveredServer, PairingOverview, PairingStatus } from '@/api/pairing';
import type { ClientDevices, ServerDevices } from '@/api/usb';
import { clientDevicesKey, clientsKey, connectionKey, pairingRequestsKey, pairingStatusKey, serverDevicesKey, serverKeysKey, settingsKey } from '@/query-keys';
import type { PendingDomain } from './pending-store';

/** How long to wait for the server's confirmation before calling a change failed. */
export const confirmTimeoutMs = 10_000;

const domain = <T, D>(value: PendingDomain<T, D>) => value;

/** The change first run sends: the role and setupComplete in one PATCH, so it is one pending change. */
export interface SetupTarget {
  role: KoppelRole;
}

/** What a save of the server connection asked for. The secret cannot be compared, so it is left out. */
export interface ConnectionTarget {
  serverUrl: string;
  serverFingerprint: string;
  clientId: string;
}

const settingLabels: Record<keyof KoppelSettings, string> = {
  role: 'Role',
  shareOnStartup: 'Share saved devices on startup',
  setupComplete: 'Setup',
  notifyPairing: 'Notify when a client wants to pair',
  notifyLink: 'Notify when the link to the server drops',
  notifyAttachFailures: 'Notify when a device does not attach',
};

/** The server stores only the scheme, host and port, in lower case, so compare those. */
const originOf = (url: string) => {
  try {
    return new URL(url).origin;
  } catch {
    return url.trim();
  }
};

/** A fingerprint as hex digits only: the server accepts colons and spaces and stores its own spelling. */
export const hexOf = (fingerprint: string | null | undefined) => (fingerprint ?? '').replace(/[^0-9a-f]/gi, '').toLowerCase();

/**
 * Every kind of change the UI sends, with the confirmed data that settles it. The query key names
 * the cache entry whose new data is judged, so a push and a refetch after a reconnect both count.
 */
export const pendingDomains = {
  settings: domain<unknown, KoppelSettings>({
    queryKey: settingsKey,
    timeoutMs: confirmTimeoutMs,
    describe: (key, target) => ({
      text:
        key === 'setup'
          ? 'Finish setup'
          : `${settingLabels[key as keyof KoppelSettings] ?? key}: ${typeof target === 'boolean' ? (target ? 'on' : 'off') : String(target)}`,
    }),
    verdict: (key, target, settings) =>
      (key === 'setup' ? settings.setupComplete && settings.role === (target as SetupTarget | undefined)?.role : settings[key as keyof KoppelSettings] === target)
        ? 'confirmed'
        : 'waiting',
  }),

  serverDevices: domain<boolean, ServerDevices>({
    queryKey: serverDevicesKey,
    timeoutMs: confirmTimeoutMs,
    describe: (busId, share) => ({ text: `${share ? 'Share' : 'Stop sharing'} bus ${busId}`, subject: busId }),
    verdict: (busId, share, next) => {
      const device = next.devices.find((d) => d.busId === busId);
      if (device?.fault) return { failed: device.fault };
      return (device ? device.state !== 'local' : false) === share ? 'confirmed' : 'waiting';
    },
  }),

  clientDevices: domain<boolean, ClientDevices>({
    queryKey: clientDevicesKey,
    // Attaching can take a while: TLS, token, import and the kernel.
    timeoutMs: 30_000,
    describe: (busId, attach) => ({ text: `${attach ? 'Attach' : 'Detach'} ${busId}`, subject: busId }),
    verdict: (busId, attach, next) => {
      const entry = next.devices.find((d) => d.device.busId === busId);
      if (entry?.fault) return { failed: entry.fault };
      return (entry?.attachedHere ?? false) === attach ? 'confirmed' : 'waiting';
    },
  }),

  clients: domain<true, ClientView[]>({
    queryKey: clientsKey,
    timeoutMs: confirmTimeoutMs,
    describe: (id, _target, queryClient) => ({
      text: `Revoke ${queryClient.getQueryData<ClientView[]>(clientsKey)?.find((c) => c.id === id)?.name ?? id}`,
      subject: id,
    }),
    verdict: (id, _target, list) => (list.some((c) => c.id === id) ? 'waiting' : 'confirmed'),
  }),

  /**
   * The certificate and the signing key, by action. The target is what the action replaces: the signing
   * key's id for `rotate`, the next certificate's fingerprint for `switch`. Settled by ServerKeysChanged.
   */
  serverKeys: domain<string | null, ServerKeysView>({
    queryKey: serverKeysKey,
    timeoutMs: confirmTimeoutMs,
    describe: (action) => ({
      text:
        action === 'rotate'
          ? 'Rotate the token signing key'
          : action === 'prepare'
            ? 'Prepare a new certificate'
            : action === 'cancel'
              ? 'Cancel the new certificate'
              : 'Switch to the new certificate',
    }),
    verdict: (action, target, keys) => {
      const done =
        action === 'rotate'
          ? keys.signingKeyId !== target
          : action === 'prepare'
            ? keys.nextFingerprint !== null
            : action === 'cancel'
              ? keys.nextFingerprint === null
              : hexOf(keys.fingerprint) === hexOf(target);
      return done ? 'confirmed' : 'waiting';
    },
  }),

  connection: domain<ConnectionTarget, ConnectionView>({
    queryKey: connectionKey,
    timeoutMs: confirmTimeoutMs,
    describe: () => ({ text: 'Save the server connection' }),
    verdict: (_key, sent, view) =>
      sent !== undefined &&
      view.serverUrl !== null &&
      originOf(view.serverUrl) === originOf(sent.serverUrl) &&
      hexOf(view.serverFingerprint) === hexOf(sent.serverFingerprint) &&
      view.clientId === sent.clientId.trim()
        ? 'confirmed'
        : 'waiting',
  }),

  /** Approve or Refuse on a request to pair, by request id. Settled once the request no longer waits in the list. */
  pairingRequests: domain<'approve' | 'deny', PairingOverview>({
    queryKey: pairingRequestsKey,
    timeoutMs: confirmTimeoutMs,
    describe: (id, action, queryClient) => {
      const name = queryClient.getQueryData<PairingOverview>(pairingRequestsKey)?.requests.find((r) => r.id === id)?.clientName ?? id;
      return { text: `${action === 'approve' ? 'Approve' : 'Refuse'} pairing with ${name}`, subject: id };
    },
    verdict: (id, _action, view) => (view.requests.some((r) => r.id === id && r.state === 'waiting') ? 'waiting' : 'confirmed'),
  }),

  /**
   * Pairing buttons. A key is a server's fingerprint (Pair) or `cancel` / `confirm`. A pairing is
   * confirmed once the request is under way, whatever it turns into: the outcome shows in the status.
   */
  pairing: domain<DiscoveredServer | null, PairingStatus>({
    queryKey: pairingStatusKey,
    timeoutMs: confirmTimeoutMs,
    describe: (key, target) => ({
      text: key === 'cancel' ? 'Cancel pairing' : key === 'confirm' ? 'Replace the server connection' : `Pair with ${target?.name ?? key}`,
      subject: key,
    }),
    verdict: (key, target, status) => {
      if (key === 'cancel') return status.state === 'idle' ? 'confirmed' : 'waiting';
      if (key === 'confirm') return status.state === 'confirm' ? 'waiting' : 'confirmed';
      return status.state !== 'idle' && !!target && status.serverUrl !== null && originOf(status.serverUrl) === originOf(target.url) ? 'confirmed' : 'waiting';
    },
  }),
};

export type PendingDomainName = keyof typeof pendingDomains;
export type PendingTarget<N extends PendingDomainName> = Parameters<(typeof pendingDomains)[N]['verdict']>[1];
export type PendingData<N extends PendingDomainName> = Parameters<(typeof pendingDomains)[N]['verdict']>[2];
