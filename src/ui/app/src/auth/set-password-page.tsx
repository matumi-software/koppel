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
export default function SetPasswordPage({ machineName, setupUntil, recheck }: { machineName: string; setupUntil: string | null; recheck: () => void }) {
  const [password, setPassword] = useState('');
  const setPasswordOnServer = useMutation({ mutationFn: setupPassword, onSettled: recheck });
  const errors = useServerErrors(setPasswordOnServer.error, 'password', 'Could not set the password.');
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
          if (!sending) setPasswordOnServer.mutate(password);
        }}
      >
        <FieldGroup label="Password">
          <Field
            label="New password"
            name="password"
            type="password"
            autoComplete="new-password"
            autoFocus
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
