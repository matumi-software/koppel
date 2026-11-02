import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AppShell } from './AppShell';

const shell = (aside: string, onAsideWidthChange = vi.fn(), defaultAsideWidth?: number) => (
  <AppShell nav={null} aside={<p>{aside}</p>} defaultAsideWidth={defaultAsideWidth} onAsideWidthChange={onAsideWidthChange} title="Devices" isMenuOpen={false} onMenuOpenChange={() => {}}>
    <p>Page</p>
  </AppShell>
);

const asideWidth = () => screen.getByRole('complementary', { name: 'Details' }).style.getPropertyValue('--kp-aside-width');

describe('AppShell', () => {
  it('keeps the aside width when its content changes', () => {
    const { rerender } = render(shell('Activity'));
    const before = asideWidth();

    rerender(shell('A device with a much longer name and many more details than the activity feed'));

    expect(asideWidth()).toBe(before);
  });

  it('resizes the aside from its edge with the arrow keys and reports the new width', async () => {
    const onAsideWidthChange = vi.fn();
    render(shell('Activity', onAsideWidthChange, 340));

    screen.getByRole('separator', { name: 'Resize the side panel' }).focus();
    await userEvent.keyboard('{ArrowLeft}');

    expect(asideWidth()).toBe('356px');
    expect(onAsideWidthChange).toHaveBeenLastCalledWith(356);
  });

  it('keeps the aside between its narrowest and widest', async () => {
    render(shell('Activity', vi.fn(), 2000));
    expect(asideWidth()).toBe('560px');

    screen.getByRole('separator', { name: 'Resize the side panel' }).focus();
    await userEvent.keyboard('{Home}');

    expect(asideWidth()).toBe('280px');
  });
});
