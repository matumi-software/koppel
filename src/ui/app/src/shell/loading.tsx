import { Skeleton } from '@heartwood/ui';

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
