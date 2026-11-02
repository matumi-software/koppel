import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

import '@koppel/react-ui/styles.css';
import './app.css';
import { App } from './app';
import { HubProvider } from './hub/hub-provider';
import { ActivityProvider } from './activity/activity-provider';

// Server state reaches the cache through hub pushes, so never refetch on a timer or on focus.
const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: Infinity, refetchOnWindowFocus: false, retry: 1 } },
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <HubProvider>
        <ActivityProvider>
          <App />
        </ActivityProvider>
      </HubProvider>
    </QueryClientProvider>
  </StrictMode>,
);
