import { useMutation } from '@tanstack/react-query';
import { SignInPage } from '@heartwood/ui';
import { setupPassword } from '@/api/auth';
import { brand } from '@/brand';
import { reasonOf, standalone } from '@/errors';
import { clock } from '@/shared/time';
import { SignedOutFrame } from '@/shell/signed-out-frame';
import { minPasswordLength } from './password-rules';

/**
 * First run, step one: the admin password. Whoever opens the UI first while setup is open sets it.
 * Signing in follows at once, so the role step comes next. Any refusal asks the server again, so a
 * setup that timed out, or that someone else finished first, shows as such. Heartwood's SignInPage asks
 * twice and keeps the action off a short or mismatched pair; the server's own refusal shows under the field.
 */
export default function SetPasswordPage({
  machineName,
  setupUntil,
  tokenRequired,
  recheck,
}: {
  machineName: string;
  setupUntil: string | null;
  tokenRequired: boolean;
  recheck: () => void;
}) {
  const setPasswordOnServer = useMutation({ mutationFn: setupPassword, onSettled: recheck });
  // A refusal of the token belongs under the token's field, any other under the password's.
  const refusal = setPasswordOnServer.isError ? standalone(reasonOf(setPasswordOnServer.error, 'Could not set the password.')) : undefined;
  const aboutToken = tokenRequired && refusal?.startsWith('The setup token') === true;
  const until = setupUntil ? clock.format(new Date(setupUntil)) : null;
  const sending = setPasswordOnServer.isPending || setPasswordOnServer.isSuccess;

  return (
    <SignedOutFrame hideBrand flush align="start">
      <SignInPage
        brand={brand}
        title={`Set up ${machineName}`}
        reason={`Step 1 of 2. Choose the admin password. ${until ? `Setup stays open until ${until}, and whoever sets the password first owns it.` : 'Whoever sets the password first owns it.'}`}
        hideUser
        newPassword
        passwordLabel="New password"
        minPasswordLength={minPasswordLength}
        setupToken={tokenRequired ? { help: "Koppel's log shows it: docker compose logs koppel, or journalctl -u koppel." } : undefined}
        tileTitle="Admin password"
        actionLabel="Set password and continue"
        embedded
        pending={sending}
        errors={{ token: aboutToken ? refusal : undefined, password: aboutToken ? undefined : refusal }}
        onSubmit={({ password, token }) => {
          if (sending) return;
          setPasswordOnServer.mutate({ password: password ?? '', setupToken: tokenRequired ? token : undefined });
        }}
      />
    </SignedOutFrame>
  );
}
