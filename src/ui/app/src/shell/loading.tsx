import type { UseQueryResult } from '@tanstack/react-query';
import { Skeleton } from '@heartwood/ui';
import { CouldNotReach } from './could-not-reach';

/** What shows while the first answer is on its way: the shape of a page, so nothing jumps. */
export function Loading() {
  return (
    <div className="kp-stack-2 kp-pad kp-loading" aria-busy="true" aria-label="Loading">
      {['60%', '45%', '55%', '40%', '50%'].map((width) => (
        <Skeleton key={width} width={width} />
      ))}
    </div>
  );
}

/**
 * What a page shows in place of a query's data until it arrives: the loading shape, or, when the request
 * failed, Could not reach Koppel with Try again. Without it a failed request would show the shape forever.
 */
export function QueryGate({ query }: { query: Pick<UseQueryResult, 'isError' | 'isFetching' | 'refetch'> }) {
  return query.isError ? <CouldNotReach isRetrying={query.isFetching} onRetry={() => void query.refetch()} /> : <Loading />;
}
