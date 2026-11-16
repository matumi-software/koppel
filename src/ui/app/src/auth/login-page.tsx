import { useMutation } from '@tanstack/react-query';
import { useState } from 'react';
import { SignInPage } from '@heartwood/ui';
import { login } from '@/api/auth';
import { brand } from '@/brand';
import { reasonOf, standalone } from '@/errors';
import { SignedOutFrame } from '@/shell/signed-out-frame';

/**
 * Sign in with the admin password. The server's answer is shown as it comes, including how long to
 * wait after too many tries. The password is never cleared silently.
 */
export default function LoginPage({ machineName, onSignedIn }: { machineName: string; onSignedIn: () => void }) {
  const signIn = useMutation({ mutationFn: login, onSuccess: onSignedIn });
  const [blank, setBlank] = useState(false);
  const busy = signIn.isPending || signIn.isSuccess;
  const error = blank ? 'Enter the admin password.' : signIn.isError ? standalone(reasonOf(signIn.error, 'Could not sign in.')) : undefined;

  return (
    <SignedOutFrame hideBrand flush align="start">
      <SignInPage
        brand={brand}
        title={`Sign in to ${machineName}`}
        reason="Enter the admin password for this computer."
        hideUser
        tileTitle="Admin password"
        embedded
        error={error}
        pending={busy}
        onSubmit={({ password }) => {
          if (busy) return;
          if (!(password ?? '').trim()) {
            setBlank(true);
            return;
          }
          setBlank(false);
          signIn.mutate(password ?? '');
        }}
        note="Forgot it? Run koppel --reset-password on this computer, then set a new one here."
      />
    </SignedOutFrame>
  );
}
