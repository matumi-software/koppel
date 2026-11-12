import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ChangePassword } from '@/settings/change-password';
import { announced, mockFetch, renderApp } from './harness';
import { busy, noContent, problem } from './auth-fixtures';

afterEach(() => vi.unstubAllGlobals());

describe('change password', () => {
  const fill = async (current: string, next: string) => {
    await userEvent.type(screen.getByLabelText('Current password'), current);
    await userEvent.type(screen.getByLabelText('New password'), next);
    await userEvent.type(screen.getByLabelText('Repeat the new password'), next);
  };

  it('completes on the REST response, with the UI header, and clears the fields', async () => {
    const { fetchMock, calls } = mockFetch({ 'POST /api/v1/auth/password': () => noContent() });
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText('Saved. Other browsers are signed out.');
    expect(calls.find((c) => c.key === 'POST /api/v1/auth/password')?.body).toEqual({ current: 'correct horse battery', next: 'another long password' });
    expect(new Headers(fetchMock.mock.calls.find(([url]) => url === '/api/v1/auth/password')![1]?.headers).get('X-Koppel-UI')).toBe('1');
    expect((screen.getByLabelText('Current password') as HTMLInputElement).value).toBe('');
  });

  it('shows the server\'s refusal and keeps what was typed', async () => {
    mockFetch({ 'POST /api/v1/auth/password': () => problem('The current password is not right.', 400) });
    renderApp(<ChangePassword locked={false} />);
    await fill('not the password', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText('Not saved: The current password is not right.');
    expect((screen.getByLabelText('New password') as HTMLInputElement).value).toBe('another long password');
  });

  it('shows the busy answer and keeps what was typed', async () => {
    mockFetch({ 'POST /api/v1/auth/password': () => problem(busy, 429) });
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText(`Not saved: ${busy}`);
    expect((screen.getByLabelText('New password') as HTMLInputElement).value).toBe('another long password');
  });

  it('checks the new password against the rules of the server before it asks the server', async () => {
    const { calls } = mockFetch({});
    renderApp(<ChangePassword locked={false} />);
    await fill('correct horse battery', 'short');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    expect(await screen.findByText('Use at least 10 characters.')).toBeTruthy();
    expect(calls.some((c) => c.key.startsWith('POST'))).toBe(false);
  });

  it('says the two new passwords differ', async () => {
    const { calls } = mockFetch({});
    renderApp(<ChangePassword locked={false} />);
    await userEvent.type(screen.getByLabelText('Current password'), 'correct horse battery');
    await userEvent.type(screen.getByLabelText('New password'), 'another long password');
    await userEvent.type(screen.getByLabelText('Repeat the new password'), 'another long passwor');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    expect(await screen.findByText('The two passwords differ.')).toBeTruthy();
    expect(calls.some((c) => c.key.startsWith('POST'))).toBe(false);
  });

  it('is disabled while the link is down', () => {
    mockFetch({});
    renderApp(<ChangePassword locked />);

    expect(screen.getByRole('button', { name: 'Change password' }).hasAttribute('disabled')).toBe(true);
  });

  it('puts a refusal of the current password under that field, and tells the live region', async () => {
    mockFetch({ 'POST /api/v1/auth/password': () => problem('The current password is not right.', 400) });
    renderApp(<ChangePassword locked={false} />);
    await fill('not the password', 'another long password');

    await userEvent.click(screen.getByRole('button', { name: 'Change password' }));

    await screen.findByText('Not saved: The current password is not right.');
    expect(screen.getByLabelText('Current password').getAttribute('aria-invalid')).toBe('true');
    expect(announced()).toBe('The password was not changed. The current password is not right.');
  });
});
