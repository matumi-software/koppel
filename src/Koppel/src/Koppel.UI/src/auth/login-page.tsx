import { useState } from 'react';
import { Button, Field } from '@koppel/react-ui';
import { login } from '@/api/auth';
import { AuthFrame } from './auth-frame';

/** Sign in with the admin password. The server's answer is shown as it comes, including how long to wait after too many tries. */
export function LoginPage({ machineName, onSignedIn }: { machineName: string; onSignedIn: () => void }) {
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string>();
  const [sending, setSending] = useState(false);

  const submit = async () => {
    setSending(true);
    setError(undefined);
    try {
      await login(password);
      onSignedIn();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : 'Could not sign in.');
      setSending(false);
    }
  };

  return (
    <AuthFrame>
      <div className="kp-page-head">
        <h1>Sign in to {machineName}</h1>
        <p>Enter the admin password for this machine.</p>
      </div>
      <form
        className="kp-card"
        aria-label="Sign in"
        style={{ display: 'flex', flexDirection: 'column', gap: 16 }}
        onSubmit={(event) => {
          event.preventDefault();
          if (password && !sending) void submit();
        }}
      >
        <Field label="Password" type="password" autoComplete="current-password" autoFocus value={password} onChange={setPassword} state={error ? 'invalid' : undefined} help={error} />
        <div className="kp-row" style={{ justifyContent: 'flex-end' }}>
          <Button variant="primary" type="submit" isSending={sending} isDisabled={!password}>
            {sending ? 'Signing in' : 'Sign in'}
          </Button>
        </div>
      </form>
      <p className="kp-note">Forgot it? Run <code>Koppel --reset-password</code> on this machine to clear it, then set a new one here.</p>
    </AuthFrame>
  );
}
