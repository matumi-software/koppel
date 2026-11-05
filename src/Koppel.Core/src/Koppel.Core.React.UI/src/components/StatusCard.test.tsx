import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { StatusCard, type StatusCardProps } from './StatusCard';

const card = (props: Partial<StatusCardProps> = {}) => render(<StatusCard state="live" machineName="lab-rack-3" onSignOut={() => {}} {...props} />);

const root = () => document.querySelector('.kp-status-card') as HTMLElement;

describe('StatusCard', () => {
  it('shows the link as live with the machine name', () => {
    card();

    expect(root().dataset.state).toBe('live');
    expect(screen.getByRole('status').textContent).toBe('Live');
    expect(screen.getByText('lab-rack-3')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it('shows the version as v0.1.1', () => {
    card({ version: '0.1.1' });

    // A screen reader hears "Koppel v0.1.1"; the card shows v0.1.1.
    expect(screen.getByText('v0.1.1').textContent).toBe('Koppel v0.1.1');
  });

  it('shows a version that is not a number as it is', () => {
    card({ version: 'dev' });

    expect(screen.getByText('dev').textContent).toBe('Koppel dev');
  });

  it('leaves the version out until it is known', () => {
    card();

    expect(document.querySelector('.kp-status-card__version')).toBeNull();
  });

  it('says changes are paused while reconnecting', () => {
    card({ state: 'reconnecting', onRetry: () => {} });

    expect(root().dataset.state).toBe('reconnecting');
    expect(screen.getByRole('status').textContent).toBe('Reconnecting');
    expect(screen.getByText('Changes are paused until the link is back.')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it('alerts when the link is down and retries on Retry', async () => {
    const onRetry = vi.fn();
    card({ state: 'down', onRetry });

    expect(root().dataset.state).toBe('down');
    expect(screen.getByRole('alert').textContent).toBe('Disconnected');
    expect(screen.getByText('Check that lab-rack-3 is running, then retry.')).toBeTruthy();

    await userEvent.click(screen.getByRole('button', { name: 'Retry' }));

    expect(onRetry).toHaveBeenCalledOnce();
  });

  it.each([
    ['connected', 'Linked to 192.168.31.122.', undefined],
    ['connecting', 'Connecting to 192.168.31.122…', 'pending'],
    ['error', 'Cannot reach 192.168.31.122.', 'pending'],
    ['unconfigured', 'No server connection yet.', 'pending'],
  ] as const)('tells a client whose server link is %s what it is linked to', (link, text, tone) => {
    card({ server: { link, name: '192.168.31.122' } });

    expect(screen.getByText(text).dataset.tone).toBe(tone);
  });

  it('names the server generically when a client does not know it yet', () => {
    card({ server: { link: 'connecting', name: null } });

    expect(screen.getByText('Connecting to the server…')).toBeTruthy();
  });

  it('signs out, and says why when that failed', async () => {
    const onSignOut = vi.fn();
    const { rerender } = card({ onSignOut });

    await userEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    expect(onSignOut).toHaveBeenCalledOnce();

    rerender(<StatusCard state="live" machineName="lab-rack-3" onSignOut={onSignOut} signOutError="the server answered 500." />);

    expect(screen.getByText('Not signed out: the server answered 500.').getAttribute('role')).toBe('status');
  });

  it('keeps Sign out available while the link is down', () => {
    card({ state: 'down', onRetry: () => {} });

    expect(screen.getByRole('button', { name: 'Sign out' }).hasAttribute('disabled')).toBe(false);
  });
});
