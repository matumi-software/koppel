import { useQuery } from '@tanstack/react-query';
import { ActivityPage as HeartwoodActivityPage } from '@heartwood/ui';
import { systemInfoOptions } from '@/queries';
import { toEvents, useActivity } from './activity-context';

/**
 * What the service confirmed or rejected, newest first, with Heartwood's search and export. Our own
 * changes still waiting show here too, as Pending. Export saves a CSV the browser downloads.
 */
export function ActivityPage() {
  const { items } = useActivity();
  const info = useQuery(systemInfoOptions());
  const name = info.data?.name ?? window.location.hostname;
  const today = new Date().toDateString();
  const older = items.some((item) => item.at.toDateString() !== today);

  return (
    <HeartwoodActivityPage
      title={`Events on ${name}`}
      description="The last 200 changes the service confirmed or rejected."
      events={toEvents(items)}
      timeWidth={older ? 160 : undefined}
      exportName={`events-on-${name}`}
    />
  );
}
