import { getJson, sendChange } from './http';

export type KoppelRole = 'server' | 'client' | 'both';

/** Mirrors Koppel.Settings.KoppelSettings on the server. */
export interface KoppelSettings {
  role: KoppelRole;
  shareOnStartup: boolean;
  listenPort: number;
  setupComplete: boolean;
}

export type SettingsPatch = Partial<KoppelSettings>;

export const getSettings = () => getJson<KoppelSettings>('/api/settings');

export const patchSettings = (patch: SettingsPatch) => sendChange('/api/settings', 'PATCH', patch);
