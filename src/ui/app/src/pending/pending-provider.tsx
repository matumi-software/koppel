import { hashKey, useQueryClient } from '@tanstack/react-query';
import { useAnnounce } from '@koppel/react-ui';
import { useEffect, useState, type ReactNode } from 'react';
import { useActivity } from '@/activity/activity-context';
import { pendingDomains } from './domains';
import { PendingContext, type PendingStores } from './pending-context';
import { PendingStore } from './pending-store';

const newStores = (): PendingStores => ({
  settings: new PendingStore(pendingDomains.settings),
  serverDevices: new PendingStore(pendingDomains.serverDevices),
  clientDevices: new PendingStore(pendingDomains.clientDevices),
  clients: new PendingStore(pendingDomains.clients),
  serverKeys: new PendingStore(pendingDomains.serverKeys),
  connection: new PendingStore(pendingDomains.connection),
  pairingWindow: new PendingStore(pendingDomains.pairingWindow),
  pairingRequests: new PendingStore(pendingDomains.pairingRequests),
  pairing: new PendingStore(pendingDomains.pairing),
});

/**
 * Owns the pending changes of the whole app and judges them whenever confirmed data lands in the
 * query cache: a live push, or a refetch after the link came back. It must sit under ActivityProvider.
 */
export function PendingProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const activity = useActivity();
  const announce = useAnnounce();
  const [stores] = useState(newStores);

  // Effects run before any change can be sent, so a store never logs to a stale activity API.
  useEffect(() => {
    for (const store of Object.values(stores)) store.attachActivity(activity);
  }, [stores, activity]);
  useEffect(() => {
    for (const store of Object.values(stores)) store.attachAnnouncer(announce);
  }, [stores, announce]);

  useEffect(() => {
    const unsubscribe = queryClient.getQueryCache().subscribe((event) => {
      if (event.type !== 'updated' || event.action.type !== 'success') return;
      for (const store of Object.values(stores)) {
        if (hashKey(store.domain.queryKey) === event.query.queryHash) store.judgeCached(event.query.state.data);
      }
    });
    return () => {
      unsubscribe();
      for (const store of Object.values(stores)) store.dispose();
    };
  }, [queryClient, stores]);

  return <PendingContext.Provider value={stores}>{children}</PendingContext.Provider>;
}
