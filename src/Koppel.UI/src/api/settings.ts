export type KoppelRole = 'server' | 'client' | 'both';

/** Mirrors Koppel.Settings.KoppelSettings on the server. */
export interface KoppelSettings {
  role: KoppelRole;
  shareOnStartup: boolean;
  requireTls: boolean;
  listenPort: number;
}

export type SettingsPatch = Partial<KoppelSettings>;

/** The server refused the change. The message is ready to show after "Not saved: ". */
export class SettingsRejectedError extends Error {}

export async function getSettings(): Promise<KoppelSettings> {
  const response = await fetch('/api/settings');
  if (!response.ok) {
    throw new Error(`Could not load settings (${response.status}).`);
  }
  return (await response.json()) as KoppelSettings;
}

/**
 * Sends a change. Resolves once the server has accepted it (202). That is not a confirmation:
 * the confirmed settings arrive over the hub as SettingsChanged.
 */
export async function patchSettings(patch: SettingsPatch): Promise<void> {
  const response = await fetch('/api/settings', {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(patch),
  });
  if (response.status === 202) {
    return;
  }
  if (response.status === 400) {
    const problem = (await response.json()) as { errors?: Record<string, string[]> };
    const first = Object.values(problem.errors ?? {})[0]?.[0];
    throw new SettingsRejectedError(first ?? 'the server rejected the change.');
  }
  throw new SettingsRejectedError(`the server answered ${response.status}.`);
}
