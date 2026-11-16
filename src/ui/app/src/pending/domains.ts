import {
  approvePairing,
  closePairingWindow,
  denyPairing,
  openPairingWindow,
  type PairingOverview,
} from '@/api/pairing';
import { cancelCertificate, closeEnrollmentCode, openEnrollmentCode, prepareCertificate, revokeClient, switchCertificate, type ClientView, type EnrollmentView, type KeysView } from '@/api/server';
import { changeServer, removeServer, type ServerSettings, type ServersView } from '@/api/servers';
import { patchSettings, type Role, type KoppelSettings, type SettingsPatch } from '@/api/settings';
import { attachDevice, detachDevice, setShared, type ClientServersView, type ServerDevices } from '@/api/usb';
import {
  clientDevicesOptions,
  clientsOptions,
  enrollmentOptions,
  pairingRequestsOptions,
  serverDevicesOptions,
  serverKeysOptions,
  serversOptions,
  settingsOptions,
} from '@/queries';
import { hexOf } from '@/shared/fingerprint';
import type { PendingDomain } from './pending-store';

/** How long to wait for the server's confirmation before calling a change failed. */
export const confirmTimeoutMs = 10_000;

/** The pending key of a device on a server, which is also its `subject` in the activity log. */
export const attachmentKey = (serverId: string, busId: string) => `${serverId}/${busId}`;

const domain = <A, D, I = void>(value: PendingDomain<A, D, I>) => value;

type Setting = keyof KoppelSettings;

/**
 * A change to one setting, or first run, which sends the role and setupComplete in one PATCH so
 * they are one pending change.
 */
export type SettingsAction = { [K in Setting]: { type: 'set'; setting: K; value: KoppelSettings[K] } }[Setting] | { type: 'setup'; role: Role };

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
  networkPort: 'Port for clients',
  networkAcceptFrom: 'Accept from',
};

const valueWord = (setting: Setting, value: KoppelSettings[Setting]) => {
  if (typeof value === 'boolean') return value ? 'on' : 'off';
  if (setting === 'networkPort') return value === null ? 'the default' : String(value);
  if (setting === 'networkAcceptFrom') return value === 'lan' ? 'this network only' : 'any network';
  return String(value);
};

/**
 * A change to one server of a client, or removing it. `change` carries only the fields being changed;
 * `name` is the server's, for the activity log. A server has one change pending at a time, whichever kind.
 */
export type ServerAction = { type: 'change'; id: string; name: string; change: ServerSettings } | { type: 'remove'; id: string; name: string };

/** Show a pairing code on this server (a new one replaces one that shows) or close it. `previous` is the code showing now, if any. */
export type EnrollmentAction = { type: 'open'; previous: string | null } | { type: 'close' };

/** The pending key of the pairing code: there is one, so one change to it is pending at a time. */
export const enrollmentKey = 'code';

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
          : `${settingLabels[action.setting]}: ${valueWord(action.setting, action.value)}`,
    }),
    verdict: (action, settings) =>
      (action.type === 'setup' ? settings.setupComplete && settings.role === action.role : settings[action.setting] === action.value) ? 'confirmed' : 'waiting',
    send: (action) => patchSettings(action.type === 'setup' ? { role: action.role, setupComplete: true } : ({ [action.setting]: action.value } satisfies Partial<SettingsPatch>)),
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

  /** Attach or detach one device of one server. Two servers can offer the same bus ID, so the key names both. */
  clientDevices: domain<{ serverId: string; busId: string; attach: boolean }, ClientServersView>({
    queryKey: clientDevicesOptions().queryKey,
    // Attaching can take a while: TLS, token, import and the kernel.
    timeoutMs: 30_000,
    keyOf: ({ serverId, busId }) => attachmentKey(serverId, busId),
    describe: ({ serverId, busId, attach }) => ({ text: `${attach ? 'Attach' : 'Detach'} ${busId}`, subject: attachmentKey(serverId, busId) }),
    verdict: ({ serverId, busId, attach }, next) => {
      const entry = next.servers.find((s) => s.id === serverId)?.devices.find((d) => d.device.busId === busId);
      if (entry?.fault) return { failed: entry.fault };
      return (entry?.attachedHere ?? false) === attach ? 'confirmed' : 'waiting';
    },
    send: ({ serverId, busId, attach }) => (attach ? attachDevice(serverId, busId) : detachDevice(serverId, busId)),
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

  serverKeys: domain<ServerKeysAction, KeysView>({
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

  /**
   * Change a server's name or link settings, or remove it. A change is settled once the saved server
   * has every field asked for, a removal once it is gone from the list. Adding a server is not here:
   * its POST answers 201 with the reason of a refusal (see `addServer`).
   */
  servers: domain<ServerAction, ServersView>({
    queryKey: serversOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: (action) => action.id,
    describe: (action) => ({ text: `${action.type === 'remove' ? 'Remove' : 'Change'} ${action.name}`, subject: action.id }),
    verdict: (action, view) => {
      const server = view.servers.find((s) => s.id === action.id);
      if (action.type === 'remove') return server ? 'waiting' : 'confirmed';
      if (!server) return { failed: 'the server is no longer there.' };
      const { name, reconnect, retryEverySeconds, giveUpAfterSeconds } = action.change;
      const same =
        (name === undefined || server.name === name) &&
        (reconnect === undefined || server.reconnect === reconnect) &&
        (retryEverySeconds === undefined || server.retryEverySeconds === retryEverySeconds) &&
        (giveUpAfterSeconds === undefined || server.giveUpAfterSeconds === giveUpAfterSeconds);
      return same ? 'confirmed' : 'waiting';
    },
    send: (action) => (action.type === 'remove' ? removeServer(action.id) : changeServer(action.id, action.change)),
  }),

  /** Show a pairing code or close it. Settled once EnrollmentChanged shows a new code, or none. */
  enrollment: domain<EnrollmentAction, EnrollmentView>({
    queryKey: enrollmentOptions().queryKey,
    timeoutMs: confirmTimeoutMs,
    keyOf: () => enrollmentKey,
    describe: (action) => ({ text: action.type === 'open' ? 'Show a pairing code' : 'Close the pairing code' }),
    verdict: (action, view) =>
      (action.type === 'open' ? view.code !== null && view.code.code !== action.previous : view.code === null) ? 'confirmed' : 'waiting',
    send: (action) => (action.type === 'open' ? openEnrollmentCode() : closeEnrollmentCode()),
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
};

export type PendingDomainName = keyof typeof pendingDomains;

type DomainOf<N extends PendingDomainName> = (typeof pendingDomains)[N];
export type PendingAction<N extends PendingDomainName> = Parameters<DomainOf<N>['keyOf']>[0];
export type PendingData<N extends PendingDomainName> = Parameters<DomainOf<N>['verdict']>[1];
export type PendingInput<N extends PendingDomainName> = Parameters<DomainOf<N>['send']>[1];
