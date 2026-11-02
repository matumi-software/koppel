import { RejectedError } from './http';

/** Mirrors Koppel.Login.UiAuthState: whether this browser is signed in, and whether a password exists yet. */
export interface AuthState {
  signedIn: boolean;
  passwordSet: boolean;
  machineName: string;
}

export async function getAuthState(): Promise<AuthState> {
  const response = await fetch('/api/auth/state');
  if (!response.ok) throw new Error(`Could not check the sign-in (${response.status}).`);
  return (await response.json()) as AuthState;
}

/**
 * The sign-in calls answer 204 and set or clear the session cookie. They use their own fetch, not
 * uiFetch: a 401 here means a wrong password, not an expired session. A refusal carries the server's
 * own sentence, ready to show.
 */
async function post(url: string, body?: unknown): Promise<void> {
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'X-Koppel-UI': '1', ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status === 204) return;
  let message = `the server answered ${response.status}.`;
  try {
    const problem = (await response.json()) as { error?: string };
    if (problem.error) message = problem.error;
  } catch {
    // Not our JSON; keep the generic message.
  }
  throw new RejectedError(message);
}

export const setupPassword = (setupCode: string, password: string) => post('/api/auth/setup', { setupCode, password });

export const login = (password: string) => post('/api/auth/login', { password });

export const logout = () => post('/api/auth/logout');

export const changePassword = (current: string, next: string) => post('/api/auth/password', { current, next });
