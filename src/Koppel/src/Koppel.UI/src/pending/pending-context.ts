import { createContext, useContext, useSyncExternalStore } from 'react';
import type { PendingData, PendingDomainName, PendingTarget } from './domains';
import type { PendingStore } from './pending-store';

export type PendingStores = { [N in PendingDomainName]: PendingStore<PendingTarget<N>, PendingData<N>> };

export const PendingContext = createContext<PendingStores | null>(null);

/**
 * The pending changes of one domain, kept above the pages: a change sent from a page is still
 * tracked, logged and settled after the page is gone. Judging happens in PendingProvider.
 */
export function usePending<N extends PendingDomainName>(name: N) {
  const stores = useContext(PendingContext);
  if (!stores) throw new Error('usePending must be used inside PendingProvider.');
  const store = stores[name];
  const { pending, failures, saved } = useSyncExternalStore(store.subscribe, store.getSnapshot);
  return { pending, failures, saved, request: store.request };
}
