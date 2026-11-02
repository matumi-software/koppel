import type { ReactNode } from 'react';
import { Radio, RadioGroup } from 'react-aria-components';

export interface RoleOption<T extends string> {
  value: T;
  title: string;
  description: string;
}

export interface RoleCardsProps<T extends string> {
  label: string;
  options: readonly RoleOption<T>[];
  value: T | null;
  /** As a stored setting: the role asked for while the server has not confirmed it. */
  pendingValue?: T;
  isDisabled?: boolean;
  onChange: (value: T) => void;
}

/**
 * Big choice cards for the machine's role. A react-aria RadioGroup, so arrow keys move between them.
 * As a stored setting the requested card shows the pending stripe until the server confirms it.
 */
export function RoleCards<T extends string>({ label, options, value, pendingValue, isDisabled, onChange }: RoleCardsProps<T>) {
  const isPending = pendingValue !== undefined;
  return (
    <RadioGroup
      className="kp-rolecards"
      data-state={isPending ? 'pending' : undefined}
      aria-label={label}
      value={isPending ? pendingValue : value}
      isDisabled={isDisabled || isPending}
      onChange={(v) => onChange(v as T)}
    >
      {options.map((option) => (
        <Radio key={option.value} value={option.value} className="kp-rolecard">
          <span className="kp-rolecard__title">
            {option.title}
            <span className="kp-rolecard__radio" aria-hidden="true" />
          </span>
          <p>{option.description}</p>
        </Radio>
      ))}
    </RadioGroup>
  );
}

export interface SetupCheck {
  id: string;
  ok: boolean;
  title: string;
  detail: ReactNode;
}

/** What this machine can and cannot do yet, with the fix for each problem. */
export function SetupChecks({ checks }: { checks: readonly SetupCheck[] }) {
  return (
    <ul className="kp-checks">
      {checks.map((check) => (
        <li key={check.id} className="kp-check" data-state={check.ok ? 'ok' : 'fault'}>
          <span className="kp-check__mark" aria-hidden="true">
            {check.ok ? '✓' : '×'}
          </span>
          <span>
            <b>{check.title}</b> {check.detail}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** Where the user is in a short flow. */
export function Steps({ steps, current }: { steps: readonly string[]; current: number }) {
  return (
    <ol className="kp-steps" aria-label="Steps">
      {steps.map((step, index) => (
        <li key={step} className="kp-step" aria-current={index === current ? 'step' : undefined}>
          <span className="kp-step__n">{index + 1}</span>
          {step}
        </li>
      ))}
    </ol>
  );
}
