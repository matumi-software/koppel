import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CheckPage } from '@/system/check-page';
import { baseRoutes, inFrame } from './fixtures';
import { json, mockFetch, renderApp } from './harness';
import { systemChecks, systemInfo } from './wire';

const both = { 'GET /api/v1/system/info': () => json({ ...systemInfo, role: 'both' }) };

afterEach(() => vi.unstubAllGlobals());

describe('the system check page', () => {
  it('shows each check with its state, its words and the advice, and counts what needs a change', async () => {
    mockFetch({ ...baseRoutes, ...both, 'GET /api/v1/system/checks': () => json(systemChecks) });
    renderApp(inFrame(<CheckPage />));

    await screen.findByText('This machine can share USB devices.');
    expect(screen.getByText('usbip-host is loaded.')).toBeTruthy();
    expect(screen.getByText('Attaching needs vhci-hcd.')).toBeTruthy();
    expect(screen.getByText('Load the vhci-hcd module.')).toBeTruthy();
    expect(screen.getByText('1 needs a change')).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'What bench-pc needs' })).toBeTruthy();
  });

  it('leaves out the check for a side this machine does not play', async () => {
    mockFetch({ ...baseRoutes, 'GET /api/v1/system/checks': () => json(systemChecks) });
    renderApp(inFrame(<CheckPage />));

    await screen.findByText('This machine can share USB devices.');
    expect(screen.queryByText('Attaching needs vhci-hcd.')).toBeNull();
    expect(screen.getByText('All ready')).toBeTruthy();
  });

  it('says all ready when every check passes, and shows the install facts once', async () => {
    mockFetch({ ...baseRoutes, 'GET /api/v1/system/checks': () => json(systemChecks.map((c) => ({ ...c, ok: true }))) });
    renderApp(inFrame(<CheckPage />));

    await screen.findByText('All ready');
    expect(screen.getAllByText('1.0.0')).toHaveLength(1);
    expect(screen.getByText('abc123def, 2026-01-01')).toBeTruthy();
  });

  it('checks again on request', async () => {
    let answer = systemChecks;
    const { count } = mockFetch({ ...baseRoutes, ...both, 'GET /api/v1/system/checks': () => json(answer) });
    renderApp(inFrame(<CheckPage />));
    await screen.findByText('1 needs a change');

    answer = systemChecks.map((c) => ({ ...c, ok: true }));
    await userEvent.click(screen.getByRole('button', { name: 'Check again' }));

    await screen.findByText('All ready');
    await waitFor(() => expect(count('GET /api/v1/system/checks')).toBe(2));
  });
});
