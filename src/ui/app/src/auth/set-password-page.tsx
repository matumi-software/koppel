import { useMutation } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, PageHeader, TextField } from '@heartwood/ui';
import { setupPassword } from '@/api/auth';
import { reasonOf, standalone } from '@/errors';
import { SignedOutFrame } from '@/shell/signed-out-frame';
import { passwordHelp, passwordProblem, repeatProblem } from './password-rules';

/**
 * First run, step one: the admin password. Whoever opens the UI first while setup is open sets it.
 * Signing in follows at once, so the role step comes next. Any refusal asks the server again, so a
 * setup that timed out, or that someone else finished first, shows as such.
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
  const [password, setPassword] = useState('');
  const [again, setAgain] = useState('');
  const [setupToken, setSetupToken] = useState('');
  const [problems, setProblems] = useState<{ password?: string; again?: string }>({});
  const setPasswordOnServer = useMutation({ mutationFn: setupPassword, onSettled: recheck });
  // A refusal of the token belongs under the token's field, any other under the password's.
  const refusal = setPasswordOnServer.isError ? standalone(reasonOf(setPasswordOnServer.error, 'Could not set the password.')) : undefined;
  const aboutToken = tokenRequired && refusal?.startsWith('The setup token') === true;
  const until = setupUntil ? new Date(setupUntil).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null;
  const sending = setPasswordOnServer.isPending || setPasswordOnServer.isSuccess;
  const passwordError = problems.password ?? (aboutToken ? undefined : refusal);

  const submit = () => {
    if (sending) return;
    const found = { password: passwordProblem(password) ?? undefined, again: repeatProblem(again, password) ?? undefined };
    setProblems(found);
    if (found.password || found.again) return;
    setPasswordOnServer.mutate({ password, setupToken: tokenRequired ? setupToken.trim() : undefined });
  };

  return (
    <SignedOutFrame>
      <form
        className="kp-page kp-form"
        aria-label="Set the admin password"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <PageHeader
          title={`Set up ${machineName}`}
          description={`Step 1 of 2. Choose the admin password. ${until ? `Setup stays open until ${until}, and whoever sets the password first owns it.` : 'Whoever sets the password first owns it.'}`}
        />
        {tokenRequired && (
          <TextField
            label="Setup token"
            autoComplete="off"
            autoFocus
            value={setupToken}
            onChange={setSetupToken}
            description="Koppel's log shows it: docker compose logs koppel, or journalctl -u koppel."
            isInvalid={aboutToken}
            errorMessage={aboutToken ? refusal : undefined}
          />
        )}
        <TextField
          label="New password"
          type="password"
          autoComplete="new-password"
          autoFocus={!tokenRequired}
          value={password}
          onChange={(value) => {
            setPassword(value);
            setProblems((p) => ({ ...p, password: undefined }));
          }}
          description={passwordHelp}
          isInvalid={!!passwordError}
          errorMessage={passwordError}
        />
        <TextField
          label="Repeat the password"
          type="password"
          autoComplete="new-password"
          value={again}
          onChange={(value) => {
            setAgain(value);
            setProblems((p) => ({ ...p, again: undefined }));
          }}
          isInvalid={!!problems.again}
          errorMessage={problems.again}
        />
        <div>
          <Button variant="primary" type="submit" isPending={sending}>
            Set password and continue
          </Button>
        </div>
      </form>
    </SignedOutFrame>
  );
}
