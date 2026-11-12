import { useMutation } from '@tanstack/react-query';
import { createContext, useCallback, useContext, useSyncExternalStore } from 'react';
import type { PendingAction, PendingData, PendingDomainName, PendingInput } from './domains';
import type { PendingStore } from './pending-store';

export type PendingStores = { [N in PendingDomainName]: PendingStore<PendingAction<N>, PendingData<N>, PendingInput<N>> };

export const PendingContext = createContext<PendingStores | null>(null);

/** What `request` takes after the action: the input of the domain's send, if it has one. */
type RequestInput<I> = [I] extends [void] ? [] : [I];

/**
 * The pending changes of one domain, kept above the pages: a change sent from a page is still
 * tracked, logged and settled after the page is gone. Judging happens in PendingProvider. `request`
 * marks the change pending in the store and sends it as a mutation; a refused request fails the change
 * at once, and the confirmation comes from the cache, never from the answer.
 */
export function usePending<N extends PendingDomainName>(name: N) {
  const stores = useContext(PendingContext);
  if (!stores) throw new Error('usePending must be used inside PendingProvider.');
  const store: PendingStores[N] = stores[name];
  const { pending, failures, saved, completed } = useSyncExternalStore(store.subscribe, store.getSnapshot);

  // The mutation's own callbacks outlive the component that sent it, so leaving the page orphans nothing.
  const mutation = useMutation({
    mutationFn: ({ action, input }: { action: PendingAction<N>; input: PendingInput<N>; sendId: number }) => store.domain.send(action, input),
    onError: (error, { action, sendId }) => store.fail(action, sendId, error instanceof Error ? error.message : 'the request failed.'),
  });
  const { mutate } = mutation;

  const request = useCallback(
    (action: PendingAction<N>, ...[input]: RequestInput<PendingInput<N>>) => {
      const sendId = store.begin(action);
      if (sendId !== undefined) mutate({ action, input: input as PendingInput<N>, sendId });
    },
    [store, mutate],
  );

  return { pending, failures, saved, completed, request };
}
