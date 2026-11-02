import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { RoleCards } from './Setup';

const roles = [
  { value: 'server', title: 'Server', description: 'Shares devices.' },
  { value: 'client', title: 'Client', description: 'Attaches devices.' },
] as const;

describe('RoleCards', () => {
  it('shows the requested role as pending and ignores presses until the server confirms it', async () => {
    const onChange = vi.fn();
    render(<RoleCards label="Role" options={roles} value="server" pendingValue="client" onChange={onChange} />);

    await userEvent.click(screen.getByRole('radio', { name: /Server/ }));

    expect(screen.getByRole('radiogroup').getAttribute('data-state')).toBe('pending');
    expect((screen.getByRole('radio', { name: /Client/ }) as HTMLInputElement).checked).toBe(true);
    expect(onChange).not.toHaveBeenCalled();
  });
});
