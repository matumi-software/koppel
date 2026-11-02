import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Switch } from './Switch';

const label = () => screen.getByText('Require TLS').closest('.kp-switch')!;

describe('Switch', () => {
  it('asks for the change without flipping itself', async () => {
    const onRequestChange = vi.fn();
    render(<Switch label="Require TLS" confirmed={false} onRequestChange={onRequestChange} />);

    await userEvent.click(screen.getByRole('switch'));

    expect(onRequestChange).toHaveBeenCalledWith(true);
    expect(label().getAttribute('data-state')).toBe('off');
  });

  it('shows the requested value as pending and ignores presses while pending', async () => {
    const onRequestChange = vi.fn();
    render(<Switch label="Require TLS" confirmed={false} pendingTarget={true} onRequestChange={onRequestChange} />);

    await userEvent.click(screen.getByRole('switch'));

    expect(label().getAttribute('data-state')).toBe('pending-on');
    expect(screen.getByText('Saving…')).toBeTruthy();
    expect(onRequestChange).not.toHaveBeenCalled();
  });

  it('shows the confirmed value and the reason after a failure', () => {
    render(<Switch label="Require TLS" confirmed={true} failure="config file is read-only" onRequestChange={() => {}} />);

    expect(label().getAttribute('data-state')).toBe('failed');
    expect((screen.getByRole('switch') as HTMLInputElement).checked).toBe(true);
    expect(screen.getByText('Not saved: config file is read-only')).toBeTruthy();
  });
});
