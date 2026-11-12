import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { AnnouncerProvider } from '@koppel/react-ui';

import '@koppel/react-ui/styles.css';
import { AuthGate } from './auth/auth-gate';
import { App } from './app';
import { LiveProvider } from './live/live-provider';
import { PendingProvider } from './pending/pending-provider';
import { ActivityProvider } from './activity/activity-provider';
import { ShellBoundary } from './shell/shell-boundary';

// Server state reaches the cache through the live push (live/push-table.ts), so never refetch on a timer or on focus.
// Everything the live push does not carry (identity, system checks) is refetched when the link comes back.
const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: Infinity, refetchOnWindowFocus: false, retry: 1 } },
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <AnnouncerProvider>
        <AuthGate>
          <LiveProvider>
            <ActivityProvider>
              <PendingProvider>
                <ShellBoundary>
                  <App />
                </ShellBoundary>
              </PendingProvider>
            </ActivityProvider>
          </LiveProvider>
        </AuthGate>
      </AnnouncerProvider>
    </QueryClientProvider>
  </StrictMode>,
);
