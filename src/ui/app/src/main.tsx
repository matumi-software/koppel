import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

import '@koppel/react-ui/styles.css';
import './app.css';
import { AuthGate } from './auth/auth-gate';
import { App } from './app';
import { HubProvider } from './hub/hub-provider';
import { HubPushes } from './hub/hub-pushes';
import { PendingProvider } from './pending/pending-provider';
import { ActivityProvider } from './activity/activity-provider';

// Server state reaches the cache through hub pushes (HubPushes), so never refetch on a timer or on focus.
// Everything the hub does not push (identity, system checks) is refetched when the link comes back.
const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: Infinity, refetchOnWindowFocus: false, retry: 1 } },
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <AuthGate>
        <HubProvider>
          <HubPushes />
          <ActivityProvider>
            <PendingProvider>
              <App />
            </PendingProvider>
          </ActivityProvider>
        </HubProvider>
      </AuthGate>
    </QueryClientProvider>
  </StrictMode>,
);
