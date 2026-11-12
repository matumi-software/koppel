import { useMutation } from '@tanstack/react-query';
import { Button, Field, FirstRunShell, Form, Note, PageHead, Row } from '@koppel/react-ui';
import { login } from '@/api/auth';
import { useServerErrors } from '@/errors';

/** Sign in with the admin password. The server's answer is shown as it comes, including how long to wait after too many tries. */
export default function LoginPage({ machineName, onSignedIn }: { machineName: string; onSignedIn: () => void }) {
  const signIn = useMutation({ mutationFn: login, onSuccess: onSignedIn });
  const errors = useServerErrors(signIn.error, 'password', 'Could not sign in.');

  return (
    <FirstRunShell title="Sign in">
      <PageHead title={`Sign in to ${machineName}`} lede="Enter the admin password for this machine." />
      <Form
        label="Sign in"
        variant="card"
        validationErrors={errors}
        onSubmit={(event) => {
          event.preventDefault();
          if (!signIn.isPending) signIn.mutate(String(new FormData(event.currentTarget).get('password')));
        }}
      >
        <Field label="Password" name="password" type="password" autoComplete="current-password" autoFocus isRequired />
        <Row justify="end">
          <Button variant="primary" type="submit" isSending={signIn.isPending || signIn.isSuccess}>
            {signIn.isPending || signIn.isSuccess ? 'Signing in' : 'Sign in'}
          </Button>
        </Row>
      </Form>
      <Note>
        Forgot it? Run <code>Koppel --reset-password</code> on this machine to clear it, then set a new one here.
      </Note>
    </FirstRunShell>
  );
}
