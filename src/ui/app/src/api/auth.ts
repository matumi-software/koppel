import type { AuthState, ChangeRequest, PasswordRequest } from './generated/wire';
import { readJson, rejection } from './http';

export type { AuthState } from './generated/wire';

export async function getAuthState(): Promise<AuthState> {
  const response = await fetch('/api/v1/auth/state');
  if (!response.ok) throw new Error(`Could not check the sign-in (${response.status}).`);
  return readJson<AuthState>(response);
}

/**
 * The sign-in calls answer 204 and set or clear the session cookie. They use their own fetch, not
 * uiFetch: a 401 here means a wrong password, not an expired session. A refusal carries the server's
 * own sentence, ready to show.
 */
async function post(url: string, body?: PasswordRequest | ChangeRequest): Promise<void> {
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'X-Koppel-UI': '1', ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status === 204) return;
  throw await rejection(response);
}

/** `setupToken` is sent only where the state says it is required. */
export const setupPassword = ({ password, setupToken }: { password: string; setupToken?: string }) =>
  post('/api/v1/auth/setup', setupToken === undefined ? { password } : { password, setupToken });

export const login = (password: string) => post('/api/v1/auth/login', { password });

export const logout = () => post('/api/v1/auth/logout');

export const changePassword = (current: string, next: string) => post('/api/v1/auth/password', { current, next });
