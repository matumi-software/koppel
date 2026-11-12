import { getJson } from './http';

/** Mirrors Koppel.SystemCheck: something this machine needs, and whether it has it. */
export interface SystemCheck {
  id: string;
  ok: boolean;
  title: string;
  detail: string;
}

export const getSystemChecks = () => getJson<SystemCheck[]>('/api/system/checks');

/** Mirrors Koppel.SystemInfo: what the side panel shows about this machine. */
export interface SystemInfo {
  name: string;
  version: string;
  startedAt: string;
  role: 'server' | 'client' | 'both';
  uiPort: number;
  httpsPort: number;
  fingerprint: string;
  os: string;
  /** True when this machine has a tray icon (the Windows app), so the UI offers its notification settings. */
  tray: boolean;
}

export const getSystemInfo = () => getJson<SystemInfo>('/api/system/info');
