import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { KeysView } from '@/api/server';
import { SecurityPage } from '@/security/security-page';
import { busy, noContent, problem } from './auth-fixtures';
import { baseRoutes, inFrame, settings } from './fixtures';
import { accepted, json, mockFetch, renderApp } from './harness';
import { clients, keysView } from './wire';

afterEach(() => vi.unstubAllGlobals());

/** The live region, read directly: an open dialog hides it from the accessibility tree. */
const liveRegion = () => document.querySelector('.hw-toast-region > .hw-visually-hidden:not([aria-live])')?.textContent?.replace(new RegExp(String.fromCharCode(0x200b), 'g'), '');

const bare: KeysView = { ...keysView, nextFingerprint: null, nextPreparedAt: null };
const routes = (keys: KeysView, role = 'server') => ({
  ...baseRoutes,
  'GET /api/v1/settings': () => json({ ...settings, role }),
  'GET /api/v1/server/keys': () => json(keys),
  'GET /api/v1/server/clients': () => json(clients),
  'POST /api/v1/server/keys/next-certificate': () => accepted(),
  'DELETE /api/v1/server/keys/next-certificate': () => accepted(),
  'PUT /api/v1/server/keys/current-certificate': () => accepted(),
});

describe('the server certificate', () => {
  it('prepares a certificate and shows who has it once the push carries it', async () => {
    const { calls } = mockFetch(routes(bare));
    const { live } = renderApp(inFrame(<SecurityPage />));

    await userEvent.click(await screen.findByRole('button', { name: 'Prepare a new certificate' }));
    await waitFor(() => expect(calls.some((c) => c.key === 'POST /api/v1/server/keys/next-certificate')).toBe(true));
    expect(screen.getByRole('button', { name: 'Prepare a new certificate' }).hasAttribute('data-pending')).toBe(true);

    await act(async () => live.emit('ServerKeysChanged', keysView));

    await screen.findByText(/Ready\. 1 of 2 clients have it; old-client has not connected since it was made\./);
    expect(screen.getByRole('button', { name: 'Switch to the new certificate' })).toBeTruthy();
  });

  it('warns about a client left behind before the switch, then settles on the new fingerprint', async () => {
    const { calls } = mockFetch(routes(keysView));
    const { live } = renderApp(inFrame(<SecurityPage />));

    await userEvent.click(await screen.findByRole('button', { name: 'Switch to the new certificate' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(within(dialog).getByText(/old-client has not received the new fingerprint and will refuse this server/)).toBeTruthy();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Switch certificate' }));

    expect(calls.some((c) => c.key === 'PUT /api/v1/server/keys/current-certificate')).toBe(true);
    await act(async () => live.emit('ServerKeysChanged', { ...bare, fingerprint: keysView.nextFingerprint! }));
    await screen.findByRole('button', { name: 'Prepare a new certificate' });
  });

  it('asks before it discards the new certificate, and sends nothing on Cancel', async () => {
    const { calls } = mockFetch(routes(keysView));
    const { live } = renderApp(inFrame(<SecurityPage />));

    await userEvent.click(await screen.findByRole('button', { name: 'Discard the new certificate' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Cancel' }));
    expect(calls.some((c) => c.key.startsWith('DELETE'))).toBe(false);

    await userEvent.click(screen.getByRole('button', { name: 'Discard the new certificate' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(calls.some((c) => c.key === 'DELETE /api/v1/server/keys/next-certificate')).toBe(true));

    await act(async () => live.emit('ServerKeysChanged', bare));
    await screen.findByRole('button', { name: 'Prepare a new certificate' });
  });

  it('shows why a step failed', async () => {
    mockFetch({ ...routes(bare), 'POST /api/v1/server/keys/next-certificate': () => new Response(null, { status: 500 }) });
    renderApp(inFrame(<SecurityPage />));

    await userEvent.click(await screen.findByRole('button', { name: 'Prepare a new certificate' }));

    await screen.findByText('the server answered 500.');
  });

  it('is left out on a client', async () => {
    mockFetch(routes(bare, 'client'));
    renderApp(inFrame(<SecurityPage />));

    await screen.findByRole('button', { name: 'Change password' });
    expect(screen.queryByText('Server certificate')).toBeNull();
  });

  it('is disabled while the link is down', async () => {
    mockFetch(routes(bare));
    renderApp(inFrame(<SecurityPage />, { locked: true }));

    expect(((await screen.findByRole('button', { name: 'Prepare a new certificate' })) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('the admin password', () => {
  const open = async () => {
    await userEvent.click(await screen.findByRole('button', { name: 'Change password' }));
    return screen.findByRole('dialog');
  };
  const fill = async (dialog: HTMLElement, current: string, next: string) => {
    await userEvent.type(within(dialog).getByLabelText('Current password'), current);
    await userEvent.type(within(dialog).getByLabelText('New password'), next);
    await userEvent.type(within(dialog).getByLabelText('Repeat the password'), next);
  };

  it('completes on the answer, with the UI header, and closes the dialog', async () => {
    const { calls, fetchMock } = mockFetch({ ...routes(bare, 'client'), 'POST /api/v1/auth/password': () => noContent() });
    renderApp(inFrame(<SecurityPage />));
    const dialog = await open();
    await fill(dialog, 'correct horse battery', 'another long password');

    await userEvent.click(within(dialog).getByRole('button', { name: 'Change password' }));

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(calls.find((c) => c.key === 'POST /api/v1/auth/password')?.body).toEqual({ current: 'correct horse battery', next: 'another long password' });
    expect(new Headers(fetchMock.mock.calls.find(([url]) => url === '/api/v1/auth/password')![1]?.headers).get('X-Koppel-UI')).toBe('1');
    await screen.findByText('Password changed. Other browsers are signed out.');
  });

  it('shows the refusal of the server under the current password and keeps what was typed', async () => {
    mockFetch({ ...routes(bare, 'client'), 'POST /api/v1/auth/password': () => problem('The current password is not right.', 400) });
    renderApp(inFrame(<SecurityPage />));
    const dialog = await open();
    await fill(dialog, 'not the password', 'another long password');

    await userEvent.click(within(dialog).getByRole('button', { name: 'Change password' }));

    await screen.findByText('The current password is not right.');
    expect((within(dialog).getByLabelText('New password') as HTMLInputElement).value).toBe('another long password');
    expect(liveRegion()).toBe('The password was not changed. The current password is not right.');
  });

  it('shows the busy answer as it is', async () => {
    mockFetch({ ...routes(bare, 'client'), 'POST /api/v1/auth/password': () => problem(busy, 429) });
    renderApp(inFrame(<SecurityPage />));
    const dialog = await open();
    await fill(dialog, 'correct horse battery', 'another long password');

    await userEvent.click(within(dialog).getByRole('button', { name: 'Change password' }));

    await screen.findByText(busy);
  });

  it('does not send a new password shorter than the rules', async () => {
    const { calls } = mockFetch(routes(bare, 'client'));
    renderApp(inFrame(<SecurityPage />));
    const dialog = await open();
    await fill(dialog, 'correct horse battery', 'short');

    expect((within(dialog).getByRole('button', { name: 'Change password' }) as HTMLButtonElement).disabled).toBe(true);
    expect(within(dialog).getByText('Use at least 10 characters.')).toBeTruthy();
    expect(calls.some((c) => c.key.startsWith('POST'))).toBe(false);
  });

  it('cannot be opened while the link is down', async () => {
    mockFetch(routes(bare, 'client'));
    renderApp(inFrame(<SecurityPage />, { locked: true }));

    expect(((await screen.findByRole('button', { name: 'Change password' })) as HTMLButtonElement).disabled).toBe(true);
  });
});
