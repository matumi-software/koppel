import { getJson, sendChange } from './http';
import type { KoppelSettings, SettingsPatch } from './generated/wire';

export type { KoppelSettings, Role, SettingsPatch } from './generated/wire';

export const getSettings = () => getJson<KoppelSettings>('/api/v1/settings');

export const patchSettings = (patch: SettingsPatch) => sendChange('/api/v1/settings', 'PATCH', patch);
