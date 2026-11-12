import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { StatusCard, type StatusCardProps } from './StatusCard';

const card = (props: Partial<StatusCardProps> = {}) => render(<StatusCard state="live" machineName="lab-rack-3" onSignOut={() => {}} {...props} />);

const root = () => document.querySelector('.kp-status-card') as HTMLElement;
const word = () => document.querySelector('.kp-status-card__state')!.textContent;

describe('StatusCard', () => {
  it('shows the link as live with the machine name', () => {
    card();

    expect(root().dataset.state).toBe('live');
    expect(word()).toBe('Live');
    expect(screen.getByText('lab-rack-3')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it('shows the version as it is given', () => {
    card({ version: 'v0.1.1' });

    // A screen reader hears "Koppel v0.1.1"; the card shows v0.1.1.
    expect(screen.getByText('v0.1.1').textContent).toBe('Koppel v0.1.1');
  });

  it('leaves the version out until it is known', () => {
    card();

    expect(document.querySelector('.kp-status-card__version')).toBeNull();
  });

  it('says changes are paused while reconnecting', () => {
    card({ state: 'reconnecting', onRetry: () => {} });

    expect(root().dataset.state).toBe('reconnecting');
    expect(word()).toBe('Reconnecting');
    expect(screen.getByText('Changes are paused until the link is back.')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it('offers Retry when the link is down and retries on Retry', async () => {
    const onRetry = vi.fn();
    card({ state: 'down', onRetry });

    expect(root().dataset.state).toBe('down');
    expect(word()).toBe('Disconnected');
    expect(screen.getByText('Check that lab-rack-3 is running, then retry.')).toBeTruthy();

    await userEvent.click(screen.getByRole('button', { name: 'Retry' }));

    expect(onRetry).toHaveBeenCalledOnce();
  });

  it.each([
    ['as it should be', undefined],
    ['pending', 'pending'],
  ] as const)('shows its detail line %s', (_name, tone) => {
    card({ detail: { text: 'Linked to 192.168.31.122.', tone } });

    expect(screen.getByText('Linked to 192.168.31.122.').dataset.tone).toBe(tone);
  });

  it('signs out, and says why when that failed', async () => {
    const onSignOut = vi.fn();
    const { rerender } = card({ onSignOut });

    await userEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    expect(onSignOut).toHaveBeenCalledOnce();

    rerender(<StatusCard state="live" machineName="lab-rack-3" onSignOut={onSignOut} signOutError="the server answered 500." />);

    expect(screen.getByText('Not signed out: the server answered 500.')).toBeTruthy();
  });

  it('keeps Sign out available while the link is down', () => {
    card({ state: 'down', onRetry: () => {} });

    expect(screen.getByRole('button', { name: 'Sign out' }).hasAttribute('disabled')).toBe(false);
  });
});
