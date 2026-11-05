import { useQuery } from '@tanstack/react-query';
import { Button, CopyValue, MiniList, PairingCode, Panel, type MiniListItem } from '@koppel/react-ui';
import { getConnection } from '@/api/client';
import {
  cancelPairing,
  confirmPairing,
  getDiscoveredServers,
  getPairingStatus,
  startPairing,
  type DiscoveredServer,
} from '@/api/pairing';
import { hostOf } from '@/devices/use-devices';
import { useMachineName } from '@/shared/use-machine-name';
import { usePending } from '@/pending/pending-context';
import { connectionKey, discoveredServersKey, pairingStatusKey } from '@/query-keys';

/** Pending keys that are not a server: the two buttons of a pairing under way. */
const cancelKey = 'cancel';
const confirmKey = 'confirm';

/**
 * Client role: Koppel servers found on this network, and pairing with one. Pair is pending until
 * PairingStatusChanged shows the request under way; the code then waits for a person to type it on
 * the server, and a successful pairing ends with ConnectionChanged like any saved connection. When
 * this machine is already connected to another server, the approved credentials wait here until
 * the person confirms replacing that connection.
 */
export function NearbyServers({ locked }: { locked: boolean }) {
  const me = useMachineName();
  const servers = useQuery({ queryKey: discoveredServersKey, queryFn: getDiscoveredServers });
  // The same cache key ConnectionSection keeps current from ConnectionChanged.
  const connection = useQuery({ queryKey: connectionKey, queryFn: getConnection });
  const linkedTo = hex(connection.data?.serverFingerprint);
  const linkedHost = hostOf(connection.data?.serverUrl)?.toLowerCase();
  const status = useQuery({ queryKey: pairingStatusKey, queryFn: getPairingStatus });
  const { pending, failures, request } = usePending('pairing');

  const current = status.data;
  const busy = Object.keys(pending).length > 0 || current?.state === 'requesting' || current?.state === 'waiting' || current?.state === 'confirm';
  const list = servers.data ?? [];
  const items: MiniListItem[] = list.map((server) => {
    const warning = warningFor(server, list, linkedTo, linkedHost);
    return {
      id: server.fingerprint,
      name: server.name,
      sub: (
        <>
          {server.address}
          {server.version ? `, Koppel ${server.version}` : ''}, fingerprint {shortFingerprint(server.fingerprint)}
          {warning && (
            <span className="app-error" role="alert">
              {' '}
              {warning}
            </span>
          )}
          {failures[server.fingerprint] && <span className="app-error"> Not paired: {failures[server.fingerprint]}</span>}
        </>
      ),
      end:
        linkedTo && hex(server.fingerprint) === linkedTo ? (
          <span className="kp-type-caption">Connected</span>
        ) : (
          <Button
            variant="secondary"
            isSending={server.fingerprint in pending}
            isDisabled={locked || busy}
            onPress={() => request(server.fingerprint, server, () => startPairing(server))}
          >
            Pair
          </Button>
        ),
    };
  });

  return (
    <Panel title="Servers on your network">
      <div className="kp-panel__body">
        {current?.state === 'waiting' && current.code ? (
          <>
            <p className="kp-note">
              Type this code on <b>{current.serverName}</b>: open Clients there and find <b>{me}</b> under Waiting to pair.
            </p>
            <PairingCode code={current.code} />
            {current.serverFingerprint && (
              <p className="kp-note">
                Server fingerprint {shortFingerprint(current.serverFingerprint)}. It should match the one on that server's Clients page.
              </p>
            )}
            <div className="app-inline">
              <Button variant="secondary" isSending={cancelKey in pending} isDisabled={locked || cancelKey in pending} onPress={() => request(cancelKey, null, cancelPairing)}>
                Cancel
              </Button>
            </div>
          </>
        ) : current?.state === 'requesting' ? (
          <p className="kp-note">Asking {current.serverName} to pair…</p>
        ) : current?.state === 'confirm' ? (
          <>
            <p className="kp-note">
              <b>{current.serverName}</b> approved this machine. {current.message}
            </p>
            {current.serverFingerprint && <CopyValue value={current.serverFingerprint} wrap />}
            <p className="kp-note">Replace your connection only if this is the server you meant to pair with. Discarding leaves the connection as it is.</p>
            <div className="app-inline">
              <Button variant="primary" isSending={confirmKey in pending} isDisabled={locked || confirmKey in pending || cancelKey in pending} onPress={() => request(confirmKey, null, confirmPairing)}>
                Replace connection
              </Button>
              <Button variant="secondary" isSending={cancelKey in pending} isDisabled={locked || confirmKey in pending || cancelKey in pending} onPress={() => request(cancelKey, null, cancelPairing)}>
                Discard
              </Button>
            </div>
          </>
        ) : (
          <>
            <p className="kp-note">Pair with a server to attach its devices here. You type a code on the server to approve this machine.</p>
            <MiniList
              items={items}
              empty="Looking for Koppel servers on this network. None found yet: a server in Docker needs host networking to be found, or enter its address below."
            />
          </>
        )}
        {current?.state === 'paired' && hex(connection.data?.serverFingerprint) === hex(current.serverFingerprint) && (
          <p className="app-ok" role="status">
            Paired with {current.serverName}. Its devices are under Devices.
          </p>
        )}
        {(current?.state === 'denied' || current?.state === 'expired' || current?.state === 'failed') && (
          <p className="app-error" role="status">
            Not paired: {current.message}
          </p>
        )}
        {(failures[cancelKey] || failures[confirmKey]) && <p className="app-error">Not done: {failures[cancelKey] ?? failures[confirmKey]}</p>}
      </div>
    </Panel>
  );
}

/**
 * Anyone on the network can announce any name, so the list says when a name cannot be trusted
 * alone: two servers with one name, or a server that wears the name of the one this machine is
 * connected to but has another fingerprint. It also says when more than one address answers with a
 * fingerprint, which a machine on two networks does, and so does one copying another's fingerprint.
 */
function warningFor(server: DiscoveredServer, all: DiscoveredServer[], linkedTo: string | undefined, linkedHost: string | undefined) {
  const name = server.name.toLowerCase();
  if (all.some((other) => other.fingerprint !== server.fingerprint && other.name.toLowerCase() === name)) {
    return 'Another server on this network has the same name but a different fingerprint. Compare fingerprints before you pair.';
  }
  if (linkedTo && hex(server.fingerprint) !== linkedTo && linkedHost && (name === linkedHost || name === linkedHost.split('.')[0])) {
    return 'This name matches the server you are connected to, but the fingerprint is different. It may be a new server, or not the one you think.';
  }
  const others = server.otherAddresses;
  if (others.length > 0) {
    return `${others.join(', ')} also ${others.length === 1 ? 'answers' : 'answer'} with this fingerprint. A server on more than one network does that; if this one is not, another machine is copying its fingerprint. Check the address before you pair.`;
  }
  return undefined;
}

/** Fingerprints compare by their hex digits alone. */
const hex = (fingerprint: string | null | undefined) => (fingerprint ?? '').replace(/[^0-9a-f]/gi, '').toUpperCase() || undefined;

/** The first eight pairs, in the colon-separated form the server's Clients page shows. */
const shortFingerprint = (fingerprint: string) => {
  const pairs = hex(fingerprint)?.match(/.{2}/g) ?? [];
  return `${pairs.slice(0, 8).join(':')}…`;
};
