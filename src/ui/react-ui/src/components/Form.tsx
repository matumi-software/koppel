import { useId, type ReactNode } from 'react';
import { Form as AriaForm, type FormProps as AriaFormProps } from 'react-aria-components';

export interface FormProps extends Omit<AriaFormProps, 'className' | 'style' | 'aria-label'> {
  /** Names the form for screen readers: it is a landmark. */
  label: string;
  /** Where the form sits: on the page, as a Card, or as the body of a Panel. */
  variant?: 'card' | 'panel';
  /** Keep the form at a comfortable width, for one inside a wide panel. */
  isNarrow?: boolean;
}

/**
 * A react-aria form. Fields inside it check their rules when the form is submitted and show the
 * server's refusals through `validationErrors` (a message per field name), which clear when the
 * person edits that field. The browser's own bubbles are off: react-aria draws the messages.
 */
export function Form({ label, variant, isNarrow, ...props }: FormProps) {
  const className = variant === 'card' ? 'kp-card kp-form' : variant === 'panel' ? 'kp-panel__body kp-form' : 'kp-form';
  return <AriaForm {...props} aria-label={label} className={className} data-width={isNarrow ? 'narrow' : undefined} />;
}

/** Fields that belong together, under one name for screen readers, with the design system's spacing. */
export function FieldGroup({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="kp-fieldgroup" role="group" aria-label={label}>
      {children}
    </div>
  );
}

export interface LabeledValueProps {
  label: string;
  /** What the value is for, or why it is missing, under it. */
  help?: ReactNode;
  children?: ReactNode;
}

/** A value that is not a text input, such as an ID with a Copy button, with a label like a Field's. */
export function LabeledValue({ label, help, children }: LabeledValueProps) {
  const id = useId();
  return (
    <div className="kp-field" role="group" aria-labelledby={id}>
      <span id={id} className="kp-field__label">
        {label}
      </span>
      {children}
      {help && <span className="kp-field__help">{help}</span>}
    </div>
  );
}
