import { hashKey, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState, type ReactNode } from 'react';
import { useActivity } from '@/activity/activity-context';
import { pendingDomains, type PendingDomainName } from './domains';
import { PendingContext, type PendingStores } from './pending-context';
import { PendingStore, type PendingDomain } from './pending-store';

const names = Object.keys(pendingDomains) as PendingDomainName[];

/**
 * Owns the pending changes of the whole app and judges them whenever confirmed data lands in the
 * query cache: a live push, or a refetch after the link came back. It must sit under ActivityProvider.
 */
export function PendingProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const activity = useActivity();

  const [stores] = useState(() => {
    const created: Record<string, unknown> = {};
    for (const name of names) created[name] = new PendingStore(pendingDomains[name] as PendingDomain<unknown, unknown>, queryClient);
    return created as PendingStores;
  });

  // Effects run before any change can be sent, so a store never logs to a stale activity API.
  useEffect(() => {
    for (const name of names) (stores[name] as unknown as PendingStore<unknown, unknown>).attachActivity(activity);
  }, [stores, activity]);

  useEffect(() => {
    const unsubscribe = queryClient.getQueryCache().subscribe((event) => {
      if (event.type !== 'updated' || event.action.type !== 'success') return;
      for (const name of names) {
        if (hashKey(pendingDomains[name].queryKey) === event.query.queryHash) (stores[name] as unknown as PendingStore<unknown, unknown>).judge(event.query.state.data);
      }
    });
    return () => {
      unsubscribe();
      names.forEach((name) => stores[name].dispose());
    };
  }, [queryClient, stores]);

  return <PendingContext.Provider value={stores}>{children}</PendingContext.Provider>;
}
