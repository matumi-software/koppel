import { getJson, sendChange } from './http';
import type { KoppelSettings, SettingsPatch } from './generated/wire';

export type { AcceptFrom, KoppelSettings, Role, SettingsPatch } from './generated/wire';

export const getSettings = () => getJson<KoppelSettings>('/api/v1/settings');

/**
 * The generated SettingsPatch makes `networkPort` required, but the server leaves a missing one as it
 * is (null clears it), so a patch is any part of it.
 */
export const patchSettings = (patch: Partial<SettingsPatch>) => sendChange('/api/v1/settings', 'PATCH', patch);
