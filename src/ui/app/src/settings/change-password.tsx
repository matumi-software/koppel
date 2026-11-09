import { useState } from 'react';
import { Button, Field, Panel } from '@koppel/react-ui';
import { changePassword } from '@/api/auth';
import { minPasswordLength } from '@/auth/set-password-page';

/**
 * Changes the admin password. Other signed-in browsers are signed out; this one stays in.
 *
 * This is the one change that completes on the REST response. A setting is confirmed by the value
 * the server pushes back after it wrote its file; a password has no value to push (it is never
 * sent back), so the 204 is the server's confirmation and its refusal text is shown as it is.
 */
export function ChangePassword({ locked }: { locked: boolean }) {
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const [again, setAgain] = useState('');
  const [sending, setSending] = useState(false);
  const [failure, setFailure] = useState<string>();
  const [saved, setSaved] = useState(false);

  const tooShort = next !== '' && next.length < minPasswordLength;
  const mismatch = again !== '' && again !== next;
  const ready = current !== '' && next.length >= minPasswordLength && again === next;

  const save = async () => {
    setSending(true);
    setFailure(undefined);
    setSaved(false);
    try {
      await changePassword(current, next);
      setCurrent('');
      setNext('');
      setAgain('');
      setSaved(true);
    } catch (error) {
      setFailure(error instanceof Error ? error.message : 'the server did not answer.');
    } finally {
      setSending(false);
    }
  };

  const note = sending ? 'Saving…' : failure ? `Not saved: ${failure}` : saved ? 'Saved. Other browsers are signed out.' : undefined;

  return (
    <Panel
      title="Password"
      meta={
        <span className="kp-panel__note" data-state={sending ? 'pending' : failure ? 'fault' : undefined} role="status">
          {note}
        </span>
      }
    >
      <form
        className="kp-panel__body"
        aria-label="Change password"
        style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 420 }}
        onSubmit={(event) => {
          event.preventDefault();
          if (ready && !sending && !locked) void save();
        }}
      >
        <Field label="Current password" type="password" autoComplete="current-password" value={current} onChange={setCurrent} isDisabled={locked || sending} />
        <Field
          label="New password"
          type="password"
          autoComplete="new-password"
          value={next}
          onChange={setNext}
          isDisabled={locked || sending}
          state={tooShort ? 'invalid' : undefined}
          help={tooShort ? `Use at least ${minPasswordLength} characters.` : `At least ${minPasswordLength} characters.`}
        />
        <Field
          label="Repeat the new password"
          type="password"
          autoComplete="new-password"
          value={again}
          onChange={setAgain}
          isDisabled={locked || sending}
          state={mismatch ? 'invalid' : undefined}
          help={mismatch ? 'The two passwords differ.' : undefined}
        />
        <div>
          <Button variant="primary" type="submit" isSending={sending} isDisabled={locked || !ready}>
            Change password
          </Button>
        </div>
      </form>
    </Panel>
  );
}
