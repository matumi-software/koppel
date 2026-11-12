import type { Problem } from './generated/wire';

/** The server refused the request. The message is ready to show after "Not saved: ". */
export class RejectedError extends Error {}

/**
 * The body of an answer as the type the caller expects. Every caller names a type from
 * `generated/wire.ts`, so a payload change in Rust fails the typecheck here; the browser does not
 * check the JSON itself.
 */
export async function readJson<T>(response: Response): Promise<T> {
  const body: T = await response.json();
  return body;
}

/**
 * The sentence a refusal carries, ready to show: the first message of a validation problem, else the
 * problem's `detail`. Undefined when the body is not a problem document.
 */
export async function problemMessage(response: Response): Promise<string | undefined> {
  try {
    const problem = await readJson<Partial<Problem>>(response);
    const first = Object.values(problem.errors ?? {})[0]?.[0];
    return first ?? problem.detail;
  } catch {
    return undefined;
  }
}

/** The refusal of a request as an error with the server's own sentence, or a generic one. */
export async function rejection(response: Response): Promise<RejectedError> {
  return new RejectedError((await problemMessage(response)) ?? `the server answered ${response.status}.`);
}

/**
 * Sends a request to Koppel's API. Anything that changes state carries X-Koppel-UI, which a
 * page on another site cannot add without the server's say-so, so the server can refuse those.
 */
export async function uiFetch(url: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set('X-Koppel-UI', '1');
  const response = await fetch(url, { ...init, headers });
  if (response.status === 401) unauthorized?.();
  return response;
}

let unauthorized: (() => void) | undefined;

/**
 * Registers what happens when the server answers 401 to an API request: the session ended, so the
 * login screen shows. Returns the way to unregister.
 */
export function onUnauthorized(handler: () => void): () => void {
  unauthorized = handler;
  return () => {
    if (unauthorized === handler) unauthorized = undefined;
  };
}

export async function getJson<T>(url: string): Promise<T> {
  const response = await fetch(url);
  if (response.status === 401) unauthorized?.();
  if (!response.ok) {
    throw new Error(`Could not load ${url} (${response.status}).`);
  }
  return readJson<T>(response);
}

/**
 * Sends a change and resolves once the server accepted it (202). That is not a confirmation:
 * the confirmed value arrives over the live push.
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
