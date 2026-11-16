import type { ReactNode } from 'react';
import { AppShell } from '@matumi/ui';
import { brand } from '@/brand';

/**
 * The window around every screen that has no navigation: sign-in, first-run setup, the setup that
 * timed out, and "could not reach Koppel". The same AppShell as the app, with no nav, so the page
 * does not jump when the app takes over. `hideBrand` is for SignInPage, which shows its own mark.
 * `flush` is for a page that fills the panel (the centred states).
 */
export function SignedOutFrame({ hideBrand, flush, children }: { hideBrand?: boolean; flush?: boolean; children: ReactNode }) {
  return (
    <AppShell platform="web" brand={brand} hideBrand={hideBrand} flush={flush}>
      {children}
    </AppShell>
  );
}
