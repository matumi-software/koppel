import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ChangePassword } from '@/settings/change-password';
import { mockFetch, renderApp } from './harness';
import { busy, noContent, problem } from './auth-fixtures';

afterEach(() => vi.unstubAllGlobals());

describe('change password', () => {
  const fill = async (current: string, next: string) => {
    await userEvent.type(screen.getByLabelText('Current password'), current);
    await userEvent.type(screen.getByLabelText('New password'), next);
    await userEvent.type(screen.getByLabelText('Repeat the new password'), next);
  };

  it('completes on the REST response, with the UI header, and clears the fields', async () => {
    const { fetchMock, calls } = mockFetch({ 'POST /api/auth/password': () => noContent() });
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText('Saved. Other browsers are signed out.');
    expect(calls.find((c) => c.key === 'POST /api/auth/password')?.body).toEqual({ current: 'correct horse battery', next: 'another long password' });
    expect(new Headers(fetchMock.mock.calls.find(([url]) => url === '/api/auth/password')![1]?.headers).get('X-Koppel-UI')).toBe('1');
    expect((screen.getByLabelText('Current password') as HTMLInputElement).value).toBe('');
  });

  it('shows the server\'s refusal and keeps what was typed', async () => {
    mockFetch({ 'POST /api/auth/password': () => problem('The current password is not right.', 400) });
    renderApp(<ChangePassword locked={false} />);
    await fill('not the password', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText('Not saved: The current password is not right.');
    expect((screen.getByLabelText('New password') as HTMLInputElement).value).toBe('another long password');
  });

  it('shows the busy answer and keeps what was typed', async () => {
    mockFetch({ 'POST /api/auth/password': () => problem(busy, 429) });
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText(`Not saved: ${busy}`);
    expect((screen.getByLabelText('New password') as HTMLInputElement).value).toBe('another long password');
  });

  it('is disabled while the link is down and until the new password is long enough', async () => {
    mockFetch({});
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'short');

    await waitFor(() => expect(screen.getByRole('button', { name: 'Change password' }).hasAttribute('disabled')).toBe(true));
  });
});
