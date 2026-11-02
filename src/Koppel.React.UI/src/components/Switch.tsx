import { useId } from 'react';
import { Switch as AriaSwitch } from 'react-aria-components';

export type SwitchState = 'off' | 'on' | 'pending-on' | 'pending-off' | 'failed';

export interface SwitchProps {
  label: string;
  /** The value the server last confirmed. */
  confirmed: boolean;
  /** The value the user asked for, while the server has not confirmed it yet. */
  pendingTarget?: boolean;
  /** Why the last change was not saved. The switch shows the confirmed value. */
  failure?: string;
  /** Caption shown when there is nothing pending or failed, such as "Saved". */
  note?: string;
  isDisabled?: boolean;
  /** Called with the requested value. Never flips the switch by itself. */
  onRequestChange: (value: boolean) => void;
}

export function switchState(confirmed: boolean, pendingTarget?: boolean, failure?: string): SwitchState {
  if (pendingTarget !== undefined) return pendingTarget ? 'pending-on' : 'pending-off';
  if (failure) return 'failed';
  return confirmed ? 'on' : 'off';
}

/**
 * Confirm-on-push switch. It shows the server's confirmed value, or the requested value with the
 * pending stripe while the server has not confirmed it.
 */
export function Switch({ label, confirmed, pendingTarget, failure, note, isDisabled, onRequestChange }: SwitchProps) {
  const noteId = useId();
  const state = switchState(confirmed, pendingTarget, failure);
  const isPending = pendingTarget !== undefined;
  const caption = isPending ? 'Saving…' : failure ? `Not saved: ${failure}` : note;

  return (
    <AriaSwitch
      className="kp-switch"
      data-state={state}
      isSelected={isPending ? pendingTarget : confirmed}
      isReadOnly={isPending}
      isDisabled={isDisabled}
      onChange={onRequestChange}
      aria-describedby={caption ? noteId : undefined}
    >
      <span className="kp-switch__track" aria-hidden="true">
        <span className="kp-switch__thumb" />
      </span>
      <span>{label}</span>
      {caption && (
        <span className="kp-switch__note" id={noteId}>
          {caption}
        </span>
      )}
    </AriaSwitch>
  );
}
