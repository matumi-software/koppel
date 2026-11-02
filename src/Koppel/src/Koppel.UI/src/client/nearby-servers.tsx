import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useState } from 'react';
import { Button, MiniList, PairingCode, Panel, type MiniListItem } from '@koppel/react-ui';
import { getConnection } from '@/api/client';
import { cancelPairing, getDiscoveredServers, getPairingStatus, startPairing, type DiscoveredServer, type PairingStatus } from '@/api/pairing';
import { useHubEvent } from '@/hub/hub-context';
import { useMachineName } from '@/shared/use-machine-name';

const serversKey = ['discovered-servers'] as const;
const pairingKey = ['pairing-status'] as const;

/**
 * Client role: Koppel servers found on this network, and pairing with one. Pair is pending until
 * PairingStatusChanged shows the request under way; the code then waits for a person to type it on
 * the server, and a successful pairing ends with ConnectionChanged like any saved connection.
 */
export function NearbyServers({ locked }: { locked: boolean }) {
  const queryClient = useQueryClient();
  const me = useMachineName();
  const servers = useQuery({ queryKey: serversKey, queryFn: getDiscoveredServers });
  // The same cache key ConnectionSection keeps current from ConnectionChanged.
  const connection = useQuery({ queryKey: ['connection'], queryFn: getConnection });
  const linkedTo = hex(connection.data?.serverFingerprint);
  const status = useQuery({ queryKey: pairingKey, queryFn: getPairingStatus });
  const [starting, setStarting] = useState<string>();
  const [failure, setFailure] = useState<string>();

  const onServers = useCallback((list: DiscoveredServer[]) => queryClient.setQueryData(serversKey, list), [queryClient]);
  useHubEvent('ServersDiscovered', onServers);

  const onStatus = useCallback(
    (next: PairingStatus) => {
      queryClient.setQueryData(pairingKey, next);
      if (next.state !== 'idle') setStarting(undefined);
    },
    [queryClient],
  );
  useHubEvent('PairingStatusChanged', onStatus);

  const pair = (server: DiscoveredServer) => {
    setStarting(server.fingerprint);
    setFailure(undefined);
    startPairing(server).catch((error: unknown) => {
      setStarting(undefined);
      setFailure(error instanceof Error ? error.message : 'the request failed.');
    });
  };

  const current = status.data;
  const busy = starting !== undefined || current?.state === 'requesting' || current?.state === 'waiting';
  const items: MiniListItem[] = (servers.data ?? []).map((server) => ({
    id: server.fingerprint,
    name: server.name,
    sub: `${server.address}${server.version ? `, Koppel ${server.version}` : ''}`,
    end:
      linkedTo && hex(server.fingerprint) === linkedTo ? (
        <span className="kp-type-caption">Connected</span>
      ) : (
        <Button variant="secondary" isSending={starting === server.fingerprint} isDisabled={locked || busy} onPress={() => pair(server)}>
          Pair
        </Button>
      ),
  }));

  return (
    <Panel title="Servers on your network">
      <div className="kp-panel__body">
        {current?.state === 'waiting' && current.code ? (
          <>
            <p className="kp-note">
              Type this code on <b>{current.serverName}</b>: open Clients there and find <b>{me}</b> under Waiting to pair.
            </p>
            <PairingCode code={current.code} />
            <div className="app-inline">
              <Button variant="secondary" isDisabled={locked} onPress={() => void cancelPairing()}>
                Cancel
              </Button>
            </div>
          </>
        ) : current?.state === 'requesting' ? (
          <p className="kp-note">Asking {current.serverName} to pair…</p>
        ) : (
          <>
            <p className="kp-note">Pair with a server to attach its devices here. You type a code on the server to approve this machine.</p>
            <MiniList
              items={items}
              empty="Looking for Koppel servers on this network. None found yet: a server in Docker needs host networking to be found, or enter its address below."
            />
          </>
        )}
        {current?.state === 'paired' && (
          <p className="app-ok" role="status">
            Paired with {current.serverName}. Its devices are under Devices.
          </p>
        )}
        {(current?.state === 'denied' || current?.state === 'expired' || current?.state === 'failed') && (
          <p className="app-error" role="status">
            Not paired: {current.message}
          </p>
        )}
        {failure && <p className="app-error">Not paired: {failure}</p>}
      </div>
    </Panel>
  );
}

/** Fingerprints compare by their hex digits alone. */
const hex = (fingerprint: string | null | undefined) => (fingerprint ?? '').replace(/[^0-9a-f]/gi, '').toUpperCase() || undefined;
