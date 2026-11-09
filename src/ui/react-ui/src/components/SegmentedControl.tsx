import { Radio, RadioGroup } from 'react-aria-components';

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
}

export interface SegmentedControlProps<T extends string> {
  label: string;
  options: readonly SegmentOption<T>[];
  /** The selected value. For a stored setting, the value the server last confirmed. */
  value: T;
  /** For a stored setting: the value asked for while the server has not confirmed it. */
  pendingValue?: T;
  isDisabled?: boolean;
  onChange: (value: T) => void;
}

/**
 * Switches between two or three options. As a view switch it changes at once; as a stored setting
 * it shows the requested option as pending until the server confirms it.
 */
export function SegmentedControl<T extends string>({ label, options, value, pendingValue, isDisabled, onChange }: SegmentedControlProps<T>) {
  const isPending = pendingValue !== undefined;
  return (
    <RadioGroup
      className="kp-segment"
      data-state={isPending ? 'pending' : undefined}
      aria-label={label}
      orientation="horizontal"
      value={isPending ? pendingValue : value}
      isDisabled={isDisabled || isPending}
      onChange={(next) => onChange(next as T)}
    >
      {options.map((option) => (
        <Radio key={option.value} value={option.value} className="kp-segment__item">
          {option.label}
        </Radio>
      ))}
    </RadioGroup>
  );
}
