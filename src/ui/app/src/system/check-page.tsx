import { useQuery } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { Button, Checklist, Facts, PageHeader, Tile, TileStack } from '@matumi/ui';
import { systemChecksOptions, systemInfoOptions } from '@/queries';
import { useNow } from '@/shared/use-now';
import { CouldNotReach } from '@/shell/could-not-reach';
import { Loading } from '@/shell/loading';

const uptime = (since: string, now: number) => {
  const minutes = Math.max(0, Math.round((now - new Date(since).getTime()) / 60_000));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h ${minutes % 60} min`;
  return `${Math.floor(hours / 24)} days`;
};

const clock = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });

/**
 * What this computer needs to share or attach USB devices, from the service's own checks, then the
 * facts about this install. Opening the page runs the checks again, and so does Check again.
 */
export function CheckPage() {
  const checks = useQuery({ ...systemChecksOptions(), refetchOnMount: 'always' });
  const info = useQuery(systemInfoOptions());
  const now = useNow();

  if (!checks.data) {
    return checks.isError ? <CouldNotReach isRetrying={checks.isFetching} onRetry={() => void checks.refetch()} /> : <Loading />;
  }

  const name = info.data?.name ?? window.location.hostname;
  const failing = checks.data.filter((c) => !c.ok).length;
  const data = info.data;

  return (
    <div className="kp-page">
      <PageHeader
        title={`What ${name} needs`}
        description={`Checked at ${clock.format(checks.dataUpdatedAt)}.`}
        actions={
          <Button variant="secondary" icon={RefreshCw} isPending={checks.isFetching} onPress={() => void checks.refetch()}>
            Check again
          </Button>
        }
      />
      <TileStack>
        <Tile title="Checks" meta={failing ? `${failing} ${failing === 1 ? 'needs' : 'need'} a change` : 'All ready'} padded>
          <Checklist
            label="Checks"
            items={checks.data.map((c) => ({
              id: c.id,
              state: c.ok ? ('ok' as const) : ('warning' as const),
              stateLabel: c.ok ? 'Ready' : 'Needs a change',
              title: c.title,
              detail: c.detail,
            }))}
          />
        </Tile>
        {data && (
          <Tile title="This install" padded>
            <Facts
              label="This install"
              items={[
                { label: 'Version', value: data.version, mono: true },
                { label: 'Build', value: `${data.commit}, ${data.built}`, mono: true },
                { label: 'Ports', value: data.uiPort === data.httpsPort ? `${data.uiPort}` : `UI ${data.uiPort}, network ${data.httpsPort}`, mono: true },
                { label: 'System', value: data.os },
                { label: 'Up for', value: uptime(data.startedAt, now) },
              ]}
            />
          </Tile>
        )}
      </TileStack>
    </div>
  );
}
