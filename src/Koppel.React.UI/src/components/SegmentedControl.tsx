import { Radio, RadioGroup } from 'react-aria-components';

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
}

export interface SegmentedControlProps<T extends string> {
  label: string;
  options: readonly SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
}

/** Switches between two or three views. Changes what the user sees, so it never goes pending. */
export function SegmentedControl<T extends string>({ label, options, value, onChange }: SegmentedControlProps<T>) {
  return (
    <RadioGroup
      className="kp-segment"
      aria-label={label}
      orientation="horizontal"
      value={value}
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
