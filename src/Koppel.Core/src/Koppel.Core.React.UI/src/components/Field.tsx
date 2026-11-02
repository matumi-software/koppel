import type { ReactNode } from 'react';
import { Input, Label, Text, TextField, type TextFieldProps } from 'react-aria-components';

export type FieldState = 'pending' | 'invalid';

export interface FieldProps extends Omit<TextFieldProps, 'className' | 'style' | 'children'> {
  label: string;
  help?: string;
  state?: FieldState;
  /** Set for identifiers: client IDs, host:port, paths. */
  mono?: boolean;
  placeholder?: string;
  /** Extra control beside the input, such as a Show button. */
  action?: ReactNode;
}

/** A labelled text input for settings and authentication. */
export function Field({ label, help, state, mono, placeholder, action, ...props }: FieldProps) {
  const caption = state === 'pending' ? 'Saving…' : help;
  return (
    <TextField {...props} className="kp-field" data-state={state} isInvalid={state === 'invalid'}>
      <Label className="kp-field__label">{label}</Label>
      <div className="kp-field__control">
        <Input className="kp-input" data-mono={mono ? 'true' : undefined} placeholder={placeholder} />
        {action}
      </div>
      {caption && (
        <Text slot={state === 'invalid' ? 'errorMessage' : 'description'} className="kp-field__help">
          {caption}
        </Text>
      )}
    </TextField>
  );
}
