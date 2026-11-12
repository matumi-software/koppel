import { putConnection, type ConnectionView } from '@/api/client';
import {
  approvePairing,
  cancelPairing,
  closePairingWindow,
  confirmPairing,
  denyPairing,
  openPairingWindow,
  startPairing,
  type DiscoveredServer,
  type PairingOverview,
  type PairingStatus,
} from '@/api/pairing';
import { cancelCertificate, prepareCertificate, revokeClient, switchCertificate, type ClientView, type ServerKeysView } from '@/api/server';
import { patchSettings, type KoppelRole, type KoppelSettings } from '@/api/settings';
import { attachDevice, detachDevice, setShared, type ClientDevices, type ServerDevices } from '@/api/usb';
import {
  clientDevicesOptions,
  clientsOptions,
  connectionOptions,
  pairingRequestsOptions,
  pairingStatusOptions,
  serverDevicesOptions,
  serverKeysOptions,
  settingsOptions,
} from '@/queries';
import { hexOf } from '@/shared/fingerprint';
import type { PendingDomain } from './pending-store';

/** How long to wait for the server's confirmation before calling a change failed. */
export const confirmTimeoutMs = 10_000;

const domain = <A, D, I = void>(value: PendingDomain<A, D, I>) => value;

/** The server stores only the scheme, host and port, in lower case, so compare those. */
const originOf = (url: string) => {
  try {
    return new URL(url).origin;
  } catch {
    return url.trim();
  }
};

type Setting = keyof KoppelSettings;

/**
 * A change to one setting, or first run, which sends the role and setupComplete in one PATCH so
 * they are one pending change.
 */
export type SettingsAction = { [K in Setting]: { type: 'set'; setting: K; value: KoppelSettings[K] } }[Setting] | { type: 'setup'; role: KoppelRole };

/** One setting changed to a value. */
export const setting = <K extends Setting>(key: K, value: KoppelSettings[K]) => ({ type: 'set', setting: key, value }) as SettingsAction;

/** The pending key of first run. A setting's own key is its name. */
export const setupKey = 'setup';

const settingLabels: Record<Setting, string> = {
  role: 'Role',
  shareOnStartup: 'Share saved devices on startup',
  setupComplete: 'Setup',
  notifyPairing: 'Notify when a client wants to pair',
  notifyLink: 'Notify when the link to the server drops',
  notifyAttachFailures: 'Notify when a device does not attach',
};

/** What a save of the server connection asked for. The secret cannot be compared, so it is the send's input instead. */
export interface ConnectionAction {
  serverUrl: string;
  serverFingerprint: string;
  clientId: string;
}

/** The pending key of a save of the server connection. */
export const connectionKey = 'connection';

/** Pairing buttons: Pair with a server, and the two buttons of a pairing under way. */
export type PairingAction = { type: 'pair'; server: DiscoveredServer } | { type: 'cancel' } | { type: 'confirm' };

/** The pending keys of the buttons of a pairing under way. A Pair button's key is the server's fingerprint. */
export const pairingKeys = { cancel: 'cancel', confirm: 'confirm' } as const;

/**
 * The certificate, by action. `switch` carries the fingerprint of the certificate it switches to.
 * Settled by ServerKeysChanged.
 */
export type ServerKeysAction = { type: 'prepare' } | { type: 'cancel' } | { type: 'switch'; nextFingerprint: string | null };

/** Allow pairing or Close pairing (or keep it open longer) on the server. */
export type PairingWindowAction = { type: 'open' } | { type: 'close' };

/** The pending key of the pairing window: there is one, so one change to it is pending at a time. */
export const pairingWindowKey = 'window';

/** Approve or Refuse on a request to pair. `name` is the client's, for the activity log. */
export type PairingRequestAction = { type: 'approve'; id: string; name: string; code: string } | { type: 'deny'; id: string; name: string };

/**
 * Every kind of change the UI sends, with the confirmed data that settles it and the request that
 * sends it. The query key names the cache entry whose new data is judged, so a push and a refetch
 * after a reconnect both count.
 */
export const pendingDomains = {
  settings: domain<SettingsAction, KoppelSettings>({
    queryKey: settingsOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: (action) => (action.type === 'setup' ? setupKey : action.setting),
    describe: (action) => ({
      text:
        action.type === 'setup'
          ? 'Finish setup'
          : `${settingLabels[action.setting]}: ${typeof action.value === 'boolean' ? (action.value ? 'on' : 'off') : String(action.value)}`,
    }),
    verdict: (action, settings) =>
      (action.type === 'setup' ? settings.setupComplete && settings.role === action.role : settings[action.setting] === action.value) ? 'confirmed' : 'waiting',
    send: (action) => patchSettings(action.type === 'setup' ? { role: action.role, setupComplete: true } : { [action.setting]: action.value }),
  }),

  serverDevices: domain<{ busId: string; share: boolean }, ServerDevices>({
    queryKey: serverDevicesOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: (action) => action.busId,
    describe: ({ busId, share }) => ({ text: `${share ? 'Share' : 'Stop sharing'} bus ${busId}`, subject: busId }),
    verdict: ({ busId, share }, next) => {
      const device = next.devices.find((d) => d.busId === busId);
      if (device?.fault) return { failed: device.fault };
      return (device ? device.state !== 'local' : false) === share ? 'confirmed' : 'waiting';
    },
    send: ({ busId, share }) => setShared(busId, share),
  }),

  clientDevices: domain<{ busId: string; attach: boolean }, ClientDevices>({
    queryKey: clientDevicesOptions().queryKey,
    // Attaching can take a while: TLS, token, import and the kernel.
    timeoutMs: 30_000,
    keyOf: (action) => action.busId,
    describe: ({ busId, attach }) => ({ text: `${attach ? 'Attach' : 'Detach'} ${busId}`, subject: busId }),
    verdict: ({ busId, attach }, next) => {
      const entry = next.devices.find((d) => d.device.busId === busId);
      if (entry?.fault) return { failed: entry.fault };
      return (entry?.attachedHere ?? false) === attach ? 'confirmed' : 'waiting';
    },
    send: ({ busId, attach }) => (attach ? attachDevice(busId) : detachDevice(busId)),
  }),

  /** Revoke a client, by id. `name` is for the activity log. */
  clients: domain<{ id: string; name: string }, ClientView[]>({
    queryKey: clientsOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: (action) => action.id,
    describe: ({ id, name }) => ({ text: `Revoke ${name}`, subject: id }),
    verdict: ({ id }, list) => (list.some((c) => c.id === id) ? 'waiting' : 'confirmed'),
    send: ({ id }) => revokeClient(id),
  }),

  serverKeys: domain<ServerKeysAction, ServerKeysView>({
    queryKey: serverKeysOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: (action) => action.type,
    describe: (action) => ({
      text: { prepare: 'Prepare a new certificate', cancel: 'Cancel the new certificate', switch: 'Switch to the new certificate' }[action.type],
    }),
    verdict: (action, keys) => {
      const done =
        action.type === 'prepare'
          ? keys.nextFingerprint !== null
          : action.type === 'cancel'
            ? keys.nextFingerprint === null
            : hexOf(keys.fingerprint) === hexOf(action.nextFingerprint);
      return done ? 'confirmed' : 'waiting';
    },
    send: (action) => ({ prepare: prepareCertificate, cancel: cancelCertificate, switch: switchCertificate })[action.type](),
  }),

  /** A save of the server connection. The input is the client secret: it goes to the server and no further. */
  connection: domain<ConnectionAction, ConnectionView, string>({
    queryKey: connectionOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: () => connectionKey,
    describe: () => ({ text: 'Save the server connection' }),
    verdict: (sent, view) =>
      view.serverUrl !== null &&
      originOf(view.serverUrl) === originOf(sent.serverUrl) &&
      hexOf(view.serverFingerprint) === hexOf(sent.serverFingerprint) &&
      view.clientId === sent.clientId.trim()
        ? 'confirmed'
        : 'waiting',
    send: (sent, clientSecret) => putConnection({ ...sent, clientSecret }),
  }),

  /** Allow pairing or Close pairing. Settled once PairingRequestsChanged says the window is open or closed. */
  pairingWindow: domain<PairingWindowAction, PairingOverview>({
    queryKey: pairingRequestsOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: () => pairingWindowKey,
    describe: (action) => ({ text: action.type === 'open' ? 'Allow pairing' : 'Close pairing' }),
    verdict: (action, view) => (view.window.open === (action.type === 'open') ? 'confirmed' : 'waiting'),
    send: (action) => (action.type === 'open' ? openPairingWindow() : closePairingWindow()),
  }),

  /** Settled once the request no longer waits in the list. */
  pairingRequests: domain<PairingRequestAction, PairingOverview>({
    queryKey: pairingRequestsOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: (action) => action.id,
    describe: (action) => ({ text: `${action.type === 'approve' ? 'Approve' : 'Refuse'} pairing with ${action.name}`, subject: action.id }),
    verdict: ({ id }, view) => (view.requests.some((r) => r.id === id && r.state === 'waiting') ? 'waiting' : 'confirmed'),
    send: (action) => (action.type === 'approve' ? approvePairing(action.id, action.code) : denyPairing(action.id)),
  }),

  /**
   * A pairing is confirmed once the request is under way, whatever it turns into: the outcome shows
   * in the status.
   */
  pairing: domain<PairingAction, PairingStatus>({
    queryKey: pairingStatusOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: (action) => (action.type === 'pair' ? action.server.fingerprint : action.type),
    describe: (action) => ({
      text: action.type === 'cancel' ? 'Cancel pairing' : action.type === 'confirm' ? 'Replace the server connection' : `Pair with ${action.server.name}`,
      subject: action.type === 'pair' ? action.server.fingerprint : action.type,
    }),
    verdict: (action, status) => {
      if (action.type === 'cancel') return status.state === 'idle' ? 'confirmed' : 'waiting';
      if (action.type === 'confirm') return status.state === 'confirm' ? 'waiting' : 'confirmed';
      return status.state !== 'idle' && status.serverUrl !== null && originOf(status.serverUrl) === originOf(action.server.url) ? 'confirmed' : 'waiting';
    },
    send: (action) => (action.type === 'pair' ? startPairing(action.server) : action.type === 'cancel' ? cancelPairing() : confirmPairing()),
  }),
};

export type PendingDomainName = keyof typeof pendingDomains;

type DomainOf<N extends PendingDomainName> = (typeof pendingDomains)[N];
export type PendingAction<N extends PendingDomainName> = Parameters<DomainOf<N>['keyOf']>[0];
export type PendingData<N extends PendingDomainName> = Parameters<DomainOf<N>['verdict']>[1];
export type PendingInput<N extends PendingDomainName> = Parameters<DomainOf<N>['send']>[1];
