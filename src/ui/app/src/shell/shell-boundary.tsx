import type { ReactNode } from 'react';
import { Button, ErrorBoundary } from '@koppel/react-ui';
import { useAuth } from '@/auth/auth-context';

/**
 * Around the whole app: a render error shows a card with Reload and Sign out, not a blank page that
 * takes Sign out with it.
 */
export function ShellBoundary({ children }: { children: ReactNode }) {
  const { signOut } = useAuth();
  return (
    <ErrorBoundary
      actions={
        <Button variant="secondary" onPress={() => void signOut().then(() => window.location.reload())}>
          Sign out
        </Button>
      }
    >
      {children}
    </ErrorBoundary>
  );
}
