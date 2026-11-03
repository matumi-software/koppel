import { useState } from 'react';
import { Button, Field, Steps } from '@koppel/react-ui';
import { setupPassword } from '@/api/auth';
import { AuthFrame } from './auth-frame';

export const minPasswordLength = 10;

/**
 * First run, step one: the admin password. Whoever opens the UI first while setup is open sets it.
 * Signing in follows at once, so the role step comes next. Any refusal asks the server again, so a
 * setup that timed out, or that someone else finished first, shows as such.
 */
export function SetPasswordPage({ machineName, setupUntil, recheck }: { machineName: string; setupUntil: string | null; recheck: () => void }) {
  const [password, setPassword] = useState('');
  const [again, setAgain] = useState('');
  const [error, setError] = useState<string>();
  const [sending, setSending] = useState(false);

  const mismatch = again !== '' && again !== password;
  const tooShort = password !== '' && password.length < minPasswordLength;
  const ready = password.length >= minPasswordLength && again === password;
  const until = setupUntil ? new Date(setupUntil).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null;

  const submit = async () => {
    setSending(true);
    setError(undefined);
    try {
      await setupPassword(password);
      recheck();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : 'Could not set the password.');
      setSending(false);
      recheck();
    }
  };

  return (
    <AuthFrame>
      <Steps steps={['Admin password', 'What this machine does', 'Clients or server']} current={0} />
      <div className="kp-page-head">
        <h1>Set up {machineName}</h1>
        <p>
          Choose the admin password for this machine.{' '}
          {until ? `Setup is open until ${until}, and whoever sets the password first owns it.` : 'Whoever sets the password first owns it.'}
        </p>
      </div>
      <form
        className="kp-card"
        aria-label="Set the admin password"
        style={{ display: 'flex', flexDirection: 'column', gap: 16 }}
        onSubmit={(event) => {
          event.preventDefault();
          if (ready && !sending) void submit();
        }}
      >
        <Field
          label="New password"
          type="password"
          autoComplete="new-password"
          autoFocus
          value={password}
          onChange={setPassword}
          state={tooShort || error ? 'invalid' : undefined}
          help={error ?? (tooShort ? `Use at least ${minPasswordLength} characters.` : `At least ${minPasswordLength} characters.`)}
        />
        <Field
          label="Repeat the password"
          type="password"
          autoComplete="new-password"
          value={again}
          onChange={setAgain}
          state={mismatch ? 'invalid' : undefined}
          help={mismatch ? 'The two passwords differ.' : undefined}
        />
        <div className="kp-row" style={{ justifyContent: 'flex-end' }}>
          <Button variant="primary" type="submit" isSending={sending} isDisabled={!ready}>
            {sending ? 'Saving' : 'Set password and continue'}
          </Button>
        </div>
      </form>
    </AuthFrame>
  );
}
