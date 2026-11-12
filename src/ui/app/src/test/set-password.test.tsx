import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { json, mockFetch } from './harness';
import { noContent, openSetup, problem, renderGate, state } from './auth-fixtures';

afterEach(() => vi.unstubAllGlobals());

describe('first-run password', () => {
  it('asks only for a password and says until when setup is open', async () => {
    mockFetch({ 'GET /api/v1/auth/state': () => json(openSetup()) });
    renderGate();

    await screen.findByRole('heading', { name: 'Set up bench-pc' });

    expect(screen.getByText(new RegExp(`Setup is open until ${new Date('2026-11-03T12:40:00Z').toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`))).toBeTruthy();
    expect(screen.queryByLabelText('Setup code')).toBeNull();
    expect(screen.queryByText(/setup-code\.txt|docker logs/)).toBeNull();
    expect(screen.queryByText('The app')).toBeNull();
  });

  it('checks the rules when the form is sent, and sends only the password once they hold', async () => {
    let passwordSet = false;
    const { calls } = mockFetch({
      'GET /api/v1/auth/state': () => json(passwordSet ? state({ signedIn: true }) : openSetup()),
      'POST /api/v1/auth/setup': () => {
        passwordSet = true;
        return noContent();
      },
    });
    renderGate();
    const button = await screen.findByRole('button', { name: 'Set password and continue' });
    await userEvent.type(screen.getByLabelText('New password'), 'short');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'different');

    await userEvent.click(button);

    expect(await screen.findByText('Use at least 10 characters.')).toBeTruthy();
    expect(screen.getByText('The two passwords differ.')).toBeTruthy();
    expect(calls.some((c) => c.key === 'POST /api/v1/auth/setup')).toBe(false);

    await userEvent.clear(screen.getByLabelText('New password'));
    await userEvent.clear(screen.getByLabelText('Repeat the password'));
    await userEvent.type(screen.getByLabelText('New password'), 'correct horse battery');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'correct horse battery');
    await userEvent.click(button);

    await screen.findByText('The app');
    expect(calls.find((c) => c.key === 'POST /api/v1/auth/setup')?.body).toEqual({ password: 'correct horse battery' });
  });

  it('asks for the setup token the log shows where the server requires one, and sends it with the password', async () => {
    let passwordSet = false;
    const { calls } = mockFetch({
      'GET /api/v1/auth/state': () => json(passwordSet ? state({ signedIn: true }) : openSetup({ setupTokenRequired: true })),
      'POST /api/v1/auth/setup': () => {
        passwordSet = true;
        return noContent();
      },
    });
    renderGate();
    await userEvent.type(await screen.findByLabelText('Setup token'), ' 0123abcd ');
    await userEvent.type(screen.getByLabelText('New password'), 'correct horse battery');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'correct horse battery');

    await userEvent.click(screen.getByRole('button', { name: 'Set password and continue' }));

    await screen.findByText('The app');
    expect(calls.find((c) => c.key === 'POST /api/v1/auth/setup')?.body).toEqual({ password: 'correct horse battery', setupToken: '0123abcd' });
  });

  it('shows a refused token under its own field and stays', async () => {
    const refusal = "The setup token is missing or not right. Copy it from Koppel's log.";
    mockFetch({
      'GET /api/v1/auth/state': () => json(openSetup({ setupTokenRequired: true })),
      'POST /api/v1/auth/setup': () => problem(refusal, 403),
    });
    renderGate();
    await userEvent.type(await screen.findByLabelText('Setup token'), 'wrong');
    await userEvent.type(screen.getByLabelText('New password'), 'correct horse battery');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'correct horse battery');

    await userEvent.click(screen.getByRole('button', { name: 'Set password and continue' }));

    await screen.findByText(refusal);
    expect(screen.getByRole('heading', { name: 'Set up bench-pc' })).toBeTruthy();
  });

  it('asks for no token where the server does not', async () => {
    mockFetch({ 'GET /api/v1/auth/state': () => json(openSetup()) });
    renderGate();
    await screen.findByLabelText('New password');
    expect(screen.queryByLabelText('Setup token')).toBeNull();
  });

  it('shows the server\'s refusal and stays while setup is still open', async () => {
    mockFetch({ 'GET /api/v1/auth/state': () => json(openSetup()), 'POST /api/v1/auth/setup': () => problem('Do not use "koppel" or "password" as the password.', 400) });
    renderGate();
    await userEvent.type(await screen.findByLabelText('New password'), 'koppel1234');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'koppel1234');

    await userEvent.click(screen.getByRole('button', { name: 'Set password and continue' }));

    await screen.findByText('Do not use "koppel" or "password" as the password.');
    expect(screen.getByRole('heading', { name: 'Set up bench-pc' })).toBeTruthy();
  });

  it('explains the timeout and how to restart once setup has closed', async () => {
    const { count } = mockFetch({ 'GET /api/v1/auth/state': () => json(state({ passwordSet: false, setupUntil: '2026-11-03T12:40:00Z' })) });
    renderGate();

    await screen.findByRole('heading', { name: 'Setup has timed out' });

    expect(screen.getByText(/Restart Koppel, then open this page again/)).toBeTruthy();
    expect(screen.getByText('sudo systemctl restart koppel')).toBeTruthy();
    expect(screen.getByText('Restart Koppel', { selector: 'strong' })).toBeTruthy();
    expect(screen.queryByLabelText('New password')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Check again' }));
    await waitFor(() => expect(count('GET /api/v1/auth/state')).toBe(2));
  });

  it('shows the timeout screen when a password comes too late', async () => {
    let closed = false;
    mockFetch({
      'GET /api/v1/auth/state': () => json(closed ? state({ passwordSet: false, setupUntil: '2026-11-03T12:40:00Z' }) : openSetup()),
      'POST /api/v1/auth/setup': () => {
        closed = true;
        return problem('Setup has timed out. Restart Koppel, then set it.', 403);
      },
    });
    renderGate();
    await userEvent.type(await screen.findByLabelText('New password'), 'correct horse battery');
    await userEvent.type(screen.getByLabelText('Repeat the password'), 'correct horse battery');

    await userEvent.click(screen.getByRole('button', { name: 'Set password and continue' }));

    await screen.findByRole('heading', { name: 'Setup has timed out' });
  });
});
