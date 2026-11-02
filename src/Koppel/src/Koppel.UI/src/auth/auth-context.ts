import { createContext, useContext } from 'react';

export interface AuthApi {
  /** Asks the server whether this browser is still signed in; the login screen shows when it is not. */
  recheck: () => void;
  signOut: () => Promise<void>;
}

export const AuthContext = createContext<AuthApi | null>(null);

export function useAuth(): AuthApi {
  const value = useContext(AuthContext);
  if (!value) throw new Error('useAuth must be used inside AuthGate.');
  return value;
}
