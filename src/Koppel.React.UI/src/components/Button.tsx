import { Button as AriaButton, type ButtonProps as AriaButtonProps } from 'react-aria-components';

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

export interface ButtonProps extends Omit<AriaButtonProps, 'className' | 'style'> {
  variant?: ButtonVariant;
  /** True while the REST call is in flight. Shows a spinner and blocks further presses. */
  isSending?: boolean;
}

/** Starts an action. The control the action changes shows pending until the server confirms. */
export function Button({ variant = 'secondary', isSending = false, isDisabled, children, ...props }: ButtonProps) {
  return (
    <AriaButton
      {...props}
      className="kp-button"
      data-variant={variant}
      data-state={isSending ? 'sending' : undefined}
      isDisabled={isDisabled}
      isPending={isSending}
    >
      {children}
    </AriaButton>
  );
}
