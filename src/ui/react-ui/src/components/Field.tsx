import type { ReactNode } from 'react';
import { Input, Label, Text, TextField, type TextFieldProps } from 'react-aria-components';

export type FieldState = 'pending' | 'invalid';

export interface FieldProps extends Omit<TextFieldProps, 'className' | 'style' | 'children'> {
  label: string;
  /** Keep the label for screen readers only, when the row around the field already names it. */
  labelHidden?: boolean;
  help?: string;
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
 * `invalid` shows the server's answer; it is not a browser validation rule. So the field marks itself
 * invalid for screen readers only (`validationBehavior="aria"`). With react-aria's default, native, the
 * input would also get a custom validity error, and the browser would refuse to submit the form again
 * until a reload, so a corrected password could never be sent.
 */
export function Field({ label, labelHidden, help, state, mono, placeholder, action, ...props }: FieldProps) {
  const caption = state === 'pending' ? 'Saving…' : help;
  return (
    <TextField {...props} className="kp-field" data-state={state} isInvalid={state === 'invalid'} validationBehavior="aria">
      <Label className={labelHidden ? 'kp-sr-only' : 'kp-field__label'}>{label}</Label>
      <div className="kp-field__control">
        <Input className="kp-input" data-mono={mono ? 'true' : undefined} placeholder={placeholder} />
        {action}
      </div>
      {caption && (
        <Text slot={state === 'invalid' ? 'errorMessage' : 'description'} className="kp-field__help" role={state === 'invalid' ? undefined : 'status'}>
          {caption}
        </Text>
      )}
    </TextField>
  );
}
