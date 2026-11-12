import { useMutation } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Field, FieldGroup, Form, Panel, PanelNote, useAnnounce } from '@koppel/react-ui';
import { changePassword } from '@/api/auth';
import { passwordHelp, passwordProblem, repeatProblem } from '@/auth/password-rules';
import { reasonOf, useServerErrors } from '@/errors';

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
  const announce = useAnnounce();

  const change = useMutation({
    mutationFn: () => changePassword(current, next),
    onSuccess: () => {
      setCurrent('');
      setNext('');
      setAgain('');
      announce('The password was changed. Other browsers are signed out.');
    },
    onError: (error) => announce(`The password was not changed. ${reasonOf(error, 'The server did not answer.')}`),
  });
  // The form has already checked the new password against the server's rules, so a refusal that
  // gets through is the server not accepting the current one; one that still names the new password
  // (the rules moved on) goes under that field.
  const refused = change.error && (passwordProblem(next) ? 'next' : 'current');
  const errors = useServerErrors(change.error, refused || undefined, 'the server did not answer.');

  const note = change.isPending
    ? 'Saving…'
    : change.isError
      ? `Not saved: ${reasonOf(change.error, 'the server did not answer.')}`
      : change.isSuccess
        ? 'Saved. Other browsers are signed out.'
        : undefined;

  return (
    <Panel title="Password" meta={<PanelNote state={change.isPending ? 'pending' : change.isError ? 'fault' : undefined}>{note}</PanelNote>}>
      <Form
        label="Change password"
        variant="panel"
        isNarrow
        validationErrors={errors}
        onSubmit={(event) => {
          event.preventDefault();
          if (!change.isPending && !locked) change.mutate();
        }}
      >
        <FieldGroup label="Passwords">
          <Field
            label="Current password"
            name="current"
            type="password"
            autoComplete="current-password"
            isRequired
            value={current}
            onChange={setCurrent}
            isDisabled={locked || change.isPending}
          />
          <Field
            label="New password"
            name="next"
            type="password"
            autoComplete="new-password"
            isRequired
            value={next}
            onChange={setNext}
            isDisabled={locked || change.isPending}
            validate={passwordProblem}
            help={passwordHelp}
          />
          <Field
            label="Repeat the new password"
            name="again"
            type="password"
            autoComplete="new-password"
            isRequired
            value={again}
            onChange={setAgain}
            isDisabled={locked || change.isPending}
            validate={(value) => repeatProblem(value, next)}
          />
        </FieldGroup>
        <div>
          <Button variant="primary" type="submit" isSending={change.isPending} isDisabled={locked}>
            Change password
          </Button>
        </div>
      </Form>
    </Panel>
  );
}
