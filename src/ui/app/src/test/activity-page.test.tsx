import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ActivityPage } from '@/activity/activity-page';
import { baseRoutes, inFrame } from './fixtures';
import { json, mockFetch, renderApp } from './harness';
import { activityEvent } from './wire';

afterEach(() => vi.unstubAllGlobals());

const ok = { ...activityEvent, id: 'b', outcome: 'succeeded' as const, summary: 'Scanner shared.', detail: null, timestampUtc: '2026-01-01T08:10:00Z' };
const routes = { ...baseRoutes, 'GET /api/v1/activity?max=200': () => json([activityEvent, ok]) };

describe('the activity page', () => {
  it('shows the server events newest first, a refusal as Rejected with the reason', async () => {
    mockFetch(routes);
    renderApp(inFrame(<ActivityPage />));

    await screen.findByText('USB Receiver could not be attached: The link dropped.');
    expect(screen.getByRole('heading', { name: 'Events on bench-pc' })).toBeTruthy();
    expect(screen.getByText('Rejected')).toBeTruthy();
    const rows = screen.getAllByRole('row').slice(1);
    expect(within(rows[0]).getByText(/USB Receiver/)).toBeTruthy();
    expect(within(rows[1]).getByText('Scanner shared.')).toBeTruthy();
  });

  it('filters by the search and says when nothing matches', async () => {
    mockFetch(routes);
    renderApp(inFrame(<ActivityPage />));
    await screen.findByText('Scanner shared.');

    await userEvent.type(screen.getByRole('searchbox'), 'scanner');
    expect(screen.queryByText(/USB Receiver/)).toBeNull();
    expect(screen.getByText('Scanner shared.')).toBeTruthy();

    await userEvent.clear(screen.getByRole('searchbox'));
    await userEvent.type(screen.getByRole('searchbox'), 'nothing like it');
    expect(screen.getByText(/No events match/)).toBeTruthy();
  });

  it('exports what the table shows as a CSV the browser downloads', async () => {
    mockFetch(routes);
    const blobs: Blob[] = [];
    vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: (blob: Blob) => (blobs.push(blob), 'blob:csv'), revokeObjectURL: () => {} }));
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    renderApp(inFrame(<ActivityPage />));
    await screen.findByText('Scanner shared.');

    await userEvent.type(screen.getByRole('searchbox'), 'scanner');
    await userEvent.click(screen.getByRole('button', { name: /Export 1 matching event to CSV/ }));

    expect(click).toHaveBeenCalledOnce();
    const csv = await blobs[0].text();
    expect(csv).toContain('Scanner shared.');
    expect(csv).not.toContain('USB Receiver');
    click.mockRestore();
  });
});
