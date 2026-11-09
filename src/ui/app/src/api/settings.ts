import { getJson, sendChange } from './http';

export type KoppelRole = 'server' | 'client' | 'both';

/** Mirrors Koppel.Settings.KoppelSettings on the server. */
export interface KoppelSettings {
  role: KoppelRole;
  shareOnStartup: boolean;
  setupComplete: boolean;
  /** Windows app: the tray icon says when a client is waiting to pair. */
  notifyPairing: boolean;
  /** Windows app: the tray icon says when the link to the server is lost, and when it is back. */
  notifyLink: boolean;
  /** Windows app: the tray icon says when a device could not be attached. */
  notifyAttachFailures: boolean;
}

export type SettingsPatch = Partial<KoppelSettings>;

export const getSettings = () => getJson<KoppelSettings>('/api/settings');

export const patchSettings = (patch: SettingsPatch) => sendChange('/api/settings', 'PATCH', patch);
