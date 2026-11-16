import type { ReactNode } from 'react';
import { AppShell } from '@heartwood/ui';
import { brand } from '@/brand';

/**
 * The window around every screen that has no navigation: sign-in, first-run setup, the setup that
 * timed out, and "could not reach Koppel". The same AppShell as the app, with no nav, so the page
 * does not jump when the app takes over. `hideBrand` is for SignInPage, which shows its own mark.
 * `flush` is for a page that fills the panel (the centred states).
 *
 * The page sits in the middle of the window, both ways, as Koppel's sign-in always has. Heartwood's
 * sign-in and a plain page both start at the panel's left edge, so this is Koppel's own override
 * (`.kp-centre` in koppel.css). A page that must start at the left passes `align="start"`.
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
