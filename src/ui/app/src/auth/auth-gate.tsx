import { useQuery, useQueryClient } from '@tanstack/react-query';
import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, type ReactNode } from 'react';
import { ErrorBoundary, Note } from '@koppel/react-ui';
import { logout, type AuthState } from '@/api/auth';
import { onUnauthorized } from '@/api/http';
import { AuthContext } from './auth-context';

// The signed-out screens load only for a browser that is not signed in: the app is most of the code.
const LoginPage = lazy(() => import('./login-page'));
const SetPasswordPage = lazy(() => import('./set-password-page'));
const SetupClosedPage = lazy(() => import('./setup-closed-page'));
import { authOptions } from '@/queries';

const setupRecheckMs = 15_000;
const authKey = authOptions().queryKey;

/**
 * Shows the app only to a signed-in browser: the login screen when there is a password, the first-run
 * password step when there is not and setup is open, and the timeout screen once it has closed.
 * Everything under it, the live link included, mounts only after sign-in and goes away on sign-out or when the
 * server answers 401.
 */
export function AuthGate({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  // While no password is set, ask again now and then: setup may time out, or someone else may set it first.
  const state = useQuery({ ...authOptions(), refetchInterval: (query) => (query.state.data?.passwordSet === false ? setupRecheckMs : false) });
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

  const loading = <Note isPadded>Loading…</Note>;

  if (!state.data) {
    return state.isError ? <Note isPadded>Could not reach Koppel. Check that it is running, then reload.</Note> : loading;
  }

  // A render error anywhere below must not leave a blank page: the boundary offers Reload, and
  // under the app its own Sign out (see App).
  if (!state.data.passwordSet) {
    return (
      <ErrorBoundary>
        <Suspense fallback={loading}>
          {state.data.setupOpen ? (
            <SetPasswordPage machineName={state.data.machineName} setupUntil={state.data.setupUntil} tokenRequired={state.data.setupTokenRequired} recheck={recheck} />
          ) : (
            <SetupClosedPage machineName={state.data.machineName} recheck={recheck} />
          )}
        </Suspense>
      </ErrorBoundary>
    );
  }

  if (!signedIn) {
    return (
      <ErrorBoundary>
        <Suspense fallback={loading}>
          <LoginPage machineName={state.data.machineName} onSignedIn={recheck} />
        </Suspense>
      </ErrorBoundary>
    );
  }

  return <AuthContext.Provider value={api}>{children}</AuthContext.Provider>;
}
