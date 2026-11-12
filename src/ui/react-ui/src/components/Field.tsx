import type { ReactNode } from 'react';
import { FieldError, Input, Label, Text, TextField, type TextFieldProps } from 'react-aria-components';

export type FieldState = 'pending' | 'invalid';

export interface FieldProps extends Omit<TextFieldProps, 'className' | 'style' | 'children'> {
  label: string;
  /** Keep the label for screen readers only, when the row around the field already names it. */
  labelHidden?: boolean;
  help?: string;
  /**
   * `pending` while a change waits for the server. `invalid` shows an answer the server gave to a
   * field that is not in a Form (inside one, pass the answer to the Form's `validationErrors`).
   */
  state?: FieldState;
  /** Set for identifiers: client IDs, host:port, paths. */
  mono?: boolean;
  placeholder?: string;
  /** Extra control beside the input, such as a Show button. */
  action?: ReactNode;
}

/**
 * A labelled text input for settings and authentication.
 *
 * Inside a Form it takes react-aria's validation: a `validate` function or a rule such as
 * `isRequired` is checked on submit, and the message shows under the field in place of the help,
 * as does the Form's `validationErrors` entry for the field's `name`.
 *
 * `state="invalid"` is for the server's answer to a field outside a Form. It marks the field invalid
 * for screen readers only (`validationBehavior="aria"`): the browser must not keep a custom validity
 * error on it, or it would refuse to send the corrected value.
 */
export function Field({ label, labelHidden, help, state, mono, placeholder, action, ...props }: FieldProps) {
  const caption = state === 'pending' ? 'Saving…' : help;
  const standalone = state === 'invalid';
  return (
    <TextField
      {...props}
      className="kp-field"
      data-state={state}
      isInvalid={standalone ? true : props.isInvalid}
      validationBehavior={standalone ? 'aria' : props.validationBehavior}
    >
      {({ isInvalid }) => (
        <>
          <Label className={labelHidden ? 'kp-sr-only' : 'kp-field__label'}>{label}</Label>
          <div className="kp-field__control">
            <Input className="kp-input" data-mono={mono ? 'true' : undefined} placeholder={placeholder} />
            {action}
          </div>
          {/* A message from validate or the Form takes the help's place; the server's answer to a lone field is the help. */}
          {caption && (standalone || !isInvalid) && (
            <Text slot={standalone ? 'errorMessage' : 'description'} className="kp-field__help">
              {caption}
            </Text>
          )}
          {!standalone && <FieldError className="kp-field__help" />}
        </>
      )}
    </TextField>
  );
}
