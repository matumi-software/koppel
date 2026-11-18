import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ToastRegion } from '@heartwood/ui';

// Heartwood's tokens, then its components, then Koppel's own layer on top. The two fonts are bundled
// here (Heartwood expects the app to), at the weights it uses.
import './fonts.css';
import '@heartwood/tokens/tokens.css';
import '@heartwood/ui/heartwood.css';
import './koppel.css';
import { AuthGate } from './auth/auth-gate';
import { App } from './app';
import { LiveProvider } from './live/live-provider';
import { PendingProvider } from './pending/pending-provider';
import { ActivityProvider } from './activity/activity-provider';
import { ShellBoundary } from './shell/shell-boundary';
import { startTheme } from './theme/theme';

// `hw` turns on Heartwood's shared focus ring, disabled treatment and reduced motion, for overlays too.
document.body.classList.add('hw');
startTheme();

// Server state reaches the cache through the live push (live/push-table.ts), so never refetch on a timer or on focus.
// Everything the live push does not carry (identity, system checks) is refetched when the link comes back.
const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: Infinity, refetchOnWindowFocus: false, retry: 1 } },
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
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
      <ToastRegion />
    </QueryClientProvider>
  </StrictMode>,
);
