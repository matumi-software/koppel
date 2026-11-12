import { getJson } from './http';
import type { SystemCheck, SystemInfo } from './generated/wire';

export type { SystemCheck, SystemInfo } from './generated/wire';

export const getSystemChecks = () => getJson<SystemCheck[]>('/api/v1/system/checks');

export const getSystemInfo = () => getJson<SystemInfo>('/api/v1/system/info');
