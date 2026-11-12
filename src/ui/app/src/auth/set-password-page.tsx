import { useMutation } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Field, FieldGroup, FirstRunShell, Form, PageHead, Row, Steps } from '@koppel/react-ui';
import { setupPassword } from '@/api/auth';
import { useServerErrors } from '@/errors';
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
  const [setupToken, setSetupToken] = useState('');
  const setPasswordOnServer = useMutation({ mutationFn: setupPassword, onSettled: recheck });
  // A refusal of the token belongs under the token's field, any other under the password's.
  const aboutToken = tokenRequired && setPasswordOnServer.error instanceof Error && setPasswordOnServer.error.message.startsWith('The setup token');
  const errors = useServerErrors(setPasswordOnServer.error, aboutToken ? 'setupToken' : 'password', 'Could not set the password.');
  const until = setupUntil ? new Date(setupUntil).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null;
  const sending = setPasswordOnServer.isPending || setPasswordOnServer.isSuccess;

  return (
    <FirstRunShell title="Set up">
      <Steps steps={['Admin password', 'What this machine does', 'Clients or server']} current={0} />
      <PageHead
        title={`Set up ${machineName}`}
        lede={`Choose the admin password for this machine. ${
          until ? `Setup is open until ${until}, and whoever sets the password first owns it.` : 'Whoever sets the password first owns it.'
        }`}
      />
      <Form
        label="Set the admin password"
        variant="card"
        validationErrors={errors}
        onSubmit={(event) => {
          event.preventDefault();
          if (!sending) setPasswordOnServer.mutate({ password, setupToken: tokenRequired ? setupToken.trim() : undefined });
        }}
      >
        {tokenRequired && (
          <FieldGroup label="Setup access">
            <Field
              label="Setup token"
              name="setupToken"
              autoComplete="off"
              autoFocus
              isRequired
              value={setupToken}
              onChange={setSetupToken}
              help="Koppel's log shows it: docker compose logs koppel, or journalctl -u koppel."
            />
          </FieldGroup>
        )}
        <FieldGroup label="Password">
          <Field
            label="New password"
            name="password"
            type="password"
            autoComplete="new-password"
            autoFocus={!tokenRequired}
            isRequired
            value={password}
            onChange={setPassword}
            validate={passwordProblem}
            help={passwordHelp}
          />
          <Field label="Repeat the password" name="again" type="password" autoComplete="new-password" isRequired validate={(again) => repeatProblem(again, password)} />
        </FieldGroup>
        <Row justify="end">
          <Button variant="primary" type="submit" isSending={sending}>
            {sending ? 'Saving' : 'Set password and continue'}
          </Button>
        </Row>
      </Form>
    </FirstRunShell>
  );
}
