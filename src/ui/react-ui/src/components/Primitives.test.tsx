import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AnnouncerProvider, useAnnounce } from './Announcer';
import { AppShell, NavGroup, NavItem } from './AppShell';
import { Drawer, DrawerSection, Holder, KeyValues } from './Drawer';
import { ErrorBoundary } from './ErrorBoundary';
import { FirstRunShell, PageHead } from './FirstRun';
import { Card, Row, Stack } from './Layout';

function Says({ message }: { message: string }) {
  const announce = useAnnounce();
  return <button onClick={() => announce(message)}>Say it</button>;
}

describe('Announcer', () => {
  it('is the one live region, and says what it is told, again when told twice', async () => {
    render(
      <AnnouncerProvider>
        <Says message="Saved." />
      </AnnouncerProvider>,
    );
    const region = screen.getByRole('status');
    expect(region.textContent).toBe('');

    await userEvent.click(screen.getByRole('button', { name: 'Say it' }));
    const first = region.firstElementChild;
    expect(region.textContent).toBe('Saved.');
    await userEvent.click(screen.getByRole('button', { name: 'Say it' }));

    expect(region.textContent).toBe('Saved.');
    expect(region.firstElementChild).not.toBe(first);
    expect(document.querySelectorAll('[role="status"], [role="alert"], [aria-live]')).toHaveLength(1);
  });

  it('does nothing without a provider, so a component works on its own', async () => {
    render(<Says message="Saved." />);

    await userEvent.click(screen.getByRole('button', { name: 'Say it' }));

    expect(document.querySelector('[role="status"]')).toBeNull();
  });
});

describe('AppShell navigation', () => {
  const shell = (onPress = vi.fn()) => (
    <AppShell
      nav={
        <NavGroup>
          <NavItem href="#devices" isCurrent onPress={onPress}>
            Devices
          </NavItem>
        </NavGroup>
      }
      title="Devices"
      isMenuOpen={false}
      onMenuOpenChange={() => {}}
    >
      <p>Page</p>
    </AppShell>
  );

  it('makes a nav item a real link with its address, marked as the current page', () => {
    render(shell());

    const link = screen.getAllByRole('link', { name: 'Devices' })[0];
    expect(link.getAttribute('href')).toBe('#devices');
    expect(link.getAttribute('aria-current')).toBe('page');
  });

  it('tells the app when a link is followed, so it can close the phone menu', async () => {
    const onPress = vi.fn();
    render(shell(onPress));

    await userEvent.click(screen.getAllByRole('link', { name: 'Devices' })[0]);

    expect(onPress).toHaveBeenCalled();
  });

  it('has a skip link that moves focus to the page without changing the address', async () => {
    window.location.hash = '#devices';
    render(shell());

    await userEvent.click(screen.getByRole('link', { name: 'Skip to the page' }));

    expect(document.activeElement).toBe(screen.getByRole('main'));
    expect(window.location.hash).toBe('#devices');
  });
});

describe('FirstRunShell and PageHead', () => {
  it('names the tab, and gives the page one h1 that can take focus', () => {
    render(
      <FirstRunShell title="Sign in">
        <PageHead title="Sign in to bench-pc" lede="Enter the password." headingRef={() => {}} />
      </FirstRunShell>,
    );

    expect(document.title).toBe('Sign in - Koppel');
    const heading = screen.getByRole('heading', { level: 1, name: 'Sign in to bench-pc' });
    expect(heading.getAttribute('tabindex')).toBe('-1');
  });
});

describe('Drawer', () => {
  it('shows the title, closes, and lays out its parts', async () => {
    const onClose = vi.fn();
    render(
      <Drawer title="Bench 1" subtitle={<span>Added today</span>} onClose={onClose}>
        <Holder state="shared" title="Free" detail="Any client can attach it." />
        <KeyValues items={[{ label: 'Bus', value: '1-2', mono: true }]} />
        <DrawerSection title="History">
          <p>Nothing yet.</p>
        </DrawerSection>
      </Drawer>,
    );

    expect(screen.getByRole('heading', { level: 2, name: 'Bench 1' })).toBeTruthy();
    expect(screen.getByText('Any client can attach it.')).toBeTruthy();
    expect(screen.getByText('Bus').nextElementSibling?.textContent).toBe('1-2');
    expect(screen.getByRole('heading', { level: 3, name: 'History' })).toBeTruthy();

    await userEvent.click(screen.getByRole('button', { name: 'Close details' }));

    expect(onClose).toHaveBeenCalledOnce();
  });
});

describe('Layout', () => {
  it('draws a card as a labelled region, and rows and stacks with the spacing of the design system', () => {
    render(
      <Card label="Add a client" gap="sm" isFlush>
        <Stack gap="lg" align="start">
          <Row justify="end" gap="xs">
            <span>One</span>
          </Row>
        </Stack>
      </Card>,
    );

    const card = screen.getByRole('region', { name: 'Add a client' });
    expect(card.dataset.gap).toBe('sm');
    expect(card.dataset.flush).toBe('true');
    expect(card.querySelector('.kp-stack')?.getAttribute('data-gap')).toBe('lg');
    expect(card.querySelector('.kp-row')?.getAttribute('data-justify')).toBe('end');
  });
});

describe('ErrorBoundary', () => {
  function Broken(): never {
    throw new Error('the list is not an array');
  }

  it('shows a card with Reload and the actions it is given, not a blank page', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    const reload = vi.fn();
    vi.stubGlobal('location', { ...window.location, reload });
    render(
      <ErrorBoundary actions={<button>Sign out</button>}>
        <Broken />
      </ErrorBoundary>,
    );

    expect(screen.getByRole('heading', { name: 'Something went wrong' })).toBeTruthy();
    expect(screen.getByText(/the list is not an array/)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Sign out' })).toBeTruthy();
    await act(async () => userEvent.click(screen.getByRole('button', { name: 'Reload' })));
    expect(reload).toHaveBeenCalledOnce();
    expect(error).toHaveBeenCalled();
    vi.unstubAllGlobals();
    error.mockRestore();
  });

  it('shows its children when nothing is wrong', () => {
    render(
      <ErrorBoundary>
        <p>Fine</p>
      </ErrorBoundary>,
    );

    expect(screen.getByText('Fine')).toBeTruthy();
  });
});
