/** The server refused the request. The message is ready to show after "Not saved: ". */
export class RejectedError extends Error {}

/** Reads the first validation message from a 400 problem response, or a generic one. */
export async function rejection(response: Response): Promise<RejectedError> {
  if (response.status === 400) {
    try {
      const problem = (await response.json()) as { errors?: Record<string, string[]> };
      const first = Object.values(problem.errors ?? {})[0]?.[0];
      if (first) return new RejectedError(first);
    } catch {
      // Not a problem document; fall through.
    }
  }
  return new RejectedError(`the server answered ${response.status}.`);
}

/**
 * Sends a request to Koppel's API. Anything that changes state carries X-Koppel-UI, which a
 * page on another site cannot add without the server's say-so, so the server can refuse those.
 */
export function uiFetch(url: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set('X-Koppel-UI', '1');
  return fetch(url, { ...init, headers });
}

export async function getJson<T>(url: string): Promise<T> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Could not load ${url} (${response.status}).`);
  }
  return (await response.json()) as T;
}

/**
 * Sends a change and resolves once the server accepted it (202). That is not a confirmation:
 * the confirmed value arrives over the hub.
 */
export async function sendChange(url: string, method: 'POST' | 'PATCH' | 'PUT' | 'DELETE', body?: unknown): Promise<void> {
  const response = await uiFetch(url, {
    method,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status !== 202) {
    throw await rejection(response);
  }
}

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
  usbPort: number;
  fingerprint: string;
  os: string;
}

export const getSystemInfo = () => getJson<SystemInfo>('/api/system/info');
