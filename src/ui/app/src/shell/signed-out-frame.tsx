import type { ReactNode } from 'react';
import { AppShell } from '@heartwood/ui';
import { brand } from '@/brand';

/**
 * The window around every screen that has no navigation: sign-in, first-run setup, the setup that
 * timed out, and "could not reach Koppel". The same AppShell as the app, with no nav, so the page
 * does not jump when the app takes over. `hideBrand` is for SignInPage, which shows its own mark.
 * `flush` is for a page that fills the panel (the centred states).
 *
 * A plain page (the first password, setup, loading) sits in the middle of the window, both ways, as
 * Koppel's sign-in always has: Koppel's own override, `.kp-centre` in koppel.css. Heartwood's SignInPage
 * and EmptyState fill a flush panel and centre themselves, so they pass `align="start"` and the wrapper
 * stays out of their way.
 */
export function SignedOutFrame({
  hideBrand,
  flush,
  align = 'centre',
  children,
}: {
  hideBrand?: boolean;
  flush?: boolean;
  align?: 'centre' | 'start';
  children: ReactNode;
}) {
  return (
    <AppShell platform="web" brand={brand} hideBrand={hideBrand} flush={flush}>
      {align === 'centre' ? <div className="kp-centre">{children}</div> : children}
    </AppShell>
  );
}
