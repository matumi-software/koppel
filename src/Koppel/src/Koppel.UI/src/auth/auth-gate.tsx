import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useRef, type ReactNode } from 'react';
import { getAuthState, logout, type AuthState } from '@/api/auth';
import { onUnauthorized } from '@/api/http';
import { authKey } from '@/query-keys';
import { AuthContext } from './auth-context';
import { LoginPage } from './login-page';
import { SetPasswordPage } from './set-password-page';

/**
 * Shows the app only to a signed-in browser: the login screen when there is a password, the first-run
 * password step when there is not. Everything under it, the hub included, mounts only after sign-in
 * and goes away on sign-out or when the server answers 401.
 */
export function AuthGate({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const state = useQuery({ queryKey: authKey, queryFn: getAuthState });
  const signedIn = state.data?.signedIn === true;

  // A recheck that lands while signing out could bring the signed-in answer back and flash the app.
  const signingOut = useRef(false);
  const recheck = useCallback(() => {
    if (!signingOut.current) void queryClient.invalidateQueries({ queryKey: authKey });
  }, [queryClient]);

  // Any request answered 401 means the session ended: show the login screen without waiting for a refetch.
  useEffect(
    () => onUnauthorized(() => queryClient.setQueryData<AuthState>(authKey, (current) => (current ? { ...current, signedIn: false } : current))),
    [queryClient],
  );

  // What the last session read must not outlive it.
  useEffect(() => {
    if (state.data && !signedIn) queryClient.removeQueries({ predicate: (query) => query.queryKey[0] !== authKey[0] });
  }, [state.data, signedIn, queryClient]);

  const api = useMemo(
    () => ({
      recheck,
      signOut: async () => {
        signingOut.current = true;
        try {
          // An auth check already in flight would land after the sign-out with the old answer.
          await queryClient.cancelQueries({ queryKey: authKey });
          await logout();
          await queryClient.cancelQueries({ queryKey: authKey });
          queryClient.setQueryData<AuthState>(authKey, (current) => (current ? { ...current, signedIn: false } : current));
        } finally {
          signingOut.current = false;
        }
      },
    }),
    [recheck, queryClient],
  );

  if (!state.data) {
    return <p className="kp-note" style={{ padding: 24 }}>{state.isError ? 'Could not reach Koppel. Check that it is running, then reload.' : 'Loading…'}</p>;
  }

  if (!state.data.passwordSet) {
    return <SetPasswordPage machineName={state.data.machineName} onDone={recheck} />;
  }

  if (!signedIn) {
    return <LoginPage machineName={state.data.machineName} onSignedIn={recheck} />;
  }

  return <AuthContext.Provider value={api}>{children}</AuthContext.Provider>;
}
