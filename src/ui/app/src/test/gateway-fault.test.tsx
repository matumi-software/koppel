import { screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '@/app';
import { baseRoutes, settings } from './fixtures';
import { json, mockFetch, renderApp } from './harness';
import { clientDevices } from './wire';

beforeEach(async () => {
  window.location.hash = '#attach';
  await new Promise((resolve) => setTimeout(resolve, 0));
});
afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
});

const clientRoutes = (gatewayFault: string | null) => ({
  ...baseRoutes,
  'GET /api/settings': () => json({ ...settings, role: 'client' }),
  'GET /api/client/devices': () => json({ ...clientDevices, link: 'connected', linkMessage: null, devices: [], gateway: false, gatewayFault }),
});

describe('the USB/IP gateway', () => {
  it('says why it is not listening on the client devices page', async () => {
    mockFetch(clientRoutes('Port 3240 is already in use.'));
    renderApp(<App />);

    expect(await screen.findByText('The USB/IP gateway is not listening: Port 3240 is already in use.')).toBeTruthy();
  });

  it('says nothing when it has no fault', async () => {
    mockFetch(clientRoutes(null));
    renderApp(<App />);

    await screen.findByText(/is not sharing any devices/);
    expect(screen.queryByText(/gateway is not listening/)).toBeNull();
  });
});

describe('the build', () => {
  it('shows the commit and the date next to the version in the side panel', async () => {
    window.location.hash = '#overview';
    mockFetch(baseRoutes);
    renderApp(<App />);

    expect(await screen.findByText('abc123def, 2026-01-01')).toBeTruthy();
  });
});
