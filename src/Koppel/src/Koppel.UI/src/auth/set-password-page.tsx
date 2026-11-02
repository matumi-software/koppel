import { useState } from 'react';
import { Button, Field, Steps } from '@koppel/react-ui';
import { setupPassword } from '@/api/auth';
import { AuthFrame } from './auth-frame';

export const minPasswordLength = 10;

/**
 * First run, step one: the one-time setup code from the machine itself and the admin password it
 * unlocks. Signing in follows at once, so the role step comes next.
 */
export function SetPasswordPage({ machineName, onDone }: { machineName: string; onDone: () => void }) {
  const [code, setCode] = useState('');
  const [password, setPassword] = useState('');
  const [again, setAgain] = useState('');
  const [error, setError] = useState<string>();
  const [sending, setSending] = useState(false);

  const mismatch = again !== '' && again !== password;
  const tooShort = password !== '' && password.length < minPasswordLength;
  const ready = code.trim() !== '' && password.length >= minPasswordLength && again === password;

  const submit = async () => {
    setSending(true);
    setError(undefined);
    try {
      await setupPassword(code, password);
      onDone();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : 'Could not set the password.');
      setSending(false);
    }
  };

  return (
    <AuthFrame>
      <Steps steps={['Admin password', 'What this machine does', 'Clients or server']} current={0} />
      <div className="kp-page-head">
        <h1>Set up {machineName}</h1>
        <p>Choose the admin password for this machine. You need the setup code Koppel made when it started.</p>
      </div>
      <section className="kp-card" aria-label="Where to find the setup code">
        <h2>Where is the setup code?</h2>
        <ul className="kp-note">
          <li>
            In Docker: run <code>docker logs &lt;container&gt;</code> and look for the line &quot;Koppel setup code&quot;.
          </li>
          <li>
            On Windows: open <code>%ProgramData%\Koppel\config\setup-code.txt</code>. Only administrators can read it.
          </li>
        </ul>
      </section>
      <form
        className="kp-card"
        aria-label="Set the admin password"
        style={{ display: 'flex', flexDirection: 'column', gap: 16 }}
        onSubmit={(event) => {
          event.preventDefault();
          if (ready && !sending) void submit();
        }}
      >
        <Field label="Setup code" mono autoComplete="off" autoFocus placeholder="XXXX-XXXX" value={code} onChange={setCode} state={error ? 'invalid' : undefined} help={error} />
        <Field
          label="New password"
          type="password"
          autoComplete="new-password"
          value={password}
          onChange={setPassword}
          state={tooShort ? 'invalid' : undefined}
          help={tooShort ? `Use at least ${minPasswordLength} characters.` : `At least ${minPasswordLength} characters.`}
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
