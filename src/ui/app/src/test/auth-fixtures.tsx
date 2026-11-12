import { QueryClientProvider } from '@tanstack/react-query';
import { render, screen } from '@testing-library/react';
import type { AuthState } from '@/api/auth';
import { AuthGate } from '@/auth/auth-gate';
import { LiveProvider } from '@/live/live-provider';
import { RecheckButton, SignOutButton } from './auth-buttons';
import { json, newQueryClient } from './harness';
import { authState } from './wire';

export const noContent = () => new Response(null, { status: 204 });
export const problem = (error: string, status: number) => json({ error }, status);

export const busy = 'The server is busy checking another password. Try again in a moment.';

export const state = (overrides: Partial<AuthState> = {}): AuthState => ({ ...authState, signedIn: false, passwordSet: true, setupOpen: false, setupTokenRequired: false, setupUntil: null, ...overrides });

/** A machine without a password, still taking its first one. */
export const openSetup = (overrides: Partial<AuthState> = {}) => state({ passwordSet: false, setupOpen: true, setupUntil: '2026-11-03T12:40:00Z', ...overrides });

export function renderGate() {
  const queryClient = newQueryClient();
  render(
    <QueryClientProvider client={queryClient}>
      <AuthGate>
        <LiveProvider>
          <p>The app</p>
          <SignOutButton />
          <RecheckButton />
        </LiveProvider>
      </AuthGate>
    </QueryClientProvider>,
  );
}

export const loginHeading = () => screen.findByRole('heading', { name: 'Sign in to bench-pc' });
