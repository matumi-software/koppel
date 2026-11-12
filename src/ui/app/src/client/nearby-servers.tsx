import { useQuery } from '@tanstack/react-query';
import { Button, Caption, CopyValue, MiniList, Note, PairingCode, Panel, PanelBody, Row, type MiniListItem } from '@koppel/react-ui';
import type { DiscoveredServer } from '@/api/pairing';
import { pairingKeys } from '@/pending/domains';
import { hexOf, shortFingerprint } from '@/shared/fingerprint';
import { hostOf } from '@/devices/use-devices';
import { useMachineName } from '@/shared/use-machine-name';
import { usePending } from '@/pending/pending-context';
import { connectionOptions, discoveredServersOptions, pairingStatusOptions } from '@/queries';

const { cancel: cancelKey, confirm: confirmKey } = pairingKeys;

/**
 * Client role: Koppel servers found on this network, and pairing with one. Pair is pending until
 * PairingStatusChanged shows the request under way; the code then waits for a person to type it on
 * the server, and a successful pairing ends with ConnectionChanged like any saved connection. When
 * this machine is already connected to another server, the approved credentials wait here until
 * the person confirms replacing that connection.
 */
export function NearbyServers({ locked }: { locked: boolean }) {
  const me = useMachineName();
  const servers = useQuery(discoveredServersOptions());
  // The same cache key ConnectionSection keeps current from ConnectionChanged.
  const connection = useQuery(connectionOptions());
  const linkedTo = hex(connection.data?.serverFingerprint);
  const linkedHost = hostOf(connection.data?.serverUrl)?.toLowerCase();
  const status = useQuery(pairingStatusOptions());
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
          {warning && <Caption tone="fault">{` ${warning}`}</Caption>}
          {failures[server.fingerprint] && <Caption tone="fault">{` Not paired: ${failures[server.fingerprint]}`}</Caption>}
        </>
      ),
      end:
        linkedTo && hex(server.fingerprint) === linkedTo ? (
          <Caption>Connected</Caption>
        ) : (
          <Button
            variant="secondary"
            isSending={server.fingerprint in pending}
            isDisabled={locked || busy}
            onPress={() => request({ type: 'pair', server })}
          >
            Pair
          </Button>
        ),
    };
  });

  return (
    <Panel title="Servers on your network">
      <PanelBody>
        {current?.state === 'waiting' && current.code ? (
          <>
            <Note>
              Type this code on <b>{current.serverName}</b>: open Clients there and find <b>{me}</b> under Waiting to pair.
            </Note>
            <PairingCode code={current.code} />
            {current.serverFingerprint && (
              <Note>Server fingerprint {shortFingerprint(current.serverFingerprint)}. It should match the one on that server's Clients page.</Note>
            )}
            <Row>
              <Button variant="secondary" isSending={cancelKey in pending} isDisabled={locked || cancelKey in pending} onPress={() => request({ type: 'cancel' })}>
                Cancel
              </Button>
            </Row>
          </>
        ) : current?.state === 'requesting' ? (
          <Note>Asking {current.serverName} to pair…</Note>
        ) : current?.state === 'confirm' ? (
          <>
            <Note>
              <b>{current.serverName}</b> approved this machine. {current.message}
            </Note>
            {current.serverFingerprint && <CopyValue value={current.serverFingerprint} wrap />}
            <Note>Replace your connection only if this is the server you meant to pair with. Discarding leaves the connection as it is.</Note>
            <Row>
              <Button variant="primary" isSending={confirmKey in pending} isDisabled={locked || confirmKey in pending || cancelKey in pending} onPress={() => request({ type: 'confirm' })}>
                Replace connection
              </Button>
              <Button variant="secondary" isSending={cancelKey in pending} isDisabled={locked || confirmKey in pending || cancelKey in pending} onPress={() => request({ type: 'cancel' })}>
                Discard
              </Button>
            </Row>
          </>
        ) : (
          <>
            <Note>Pair with a server to attach its devices here. You type a code on the server to approve this machine.</Note>
            <MiniList
              items={items}
              empty="Looking for Koppel servers on this network. None found yet: a server in Docker needs host networking to be found, or enter its address below."
            />
          </>
        )}
        {current?.state === 'paired' && hex(connection.data?.serverFingerprint) === hex(current.serverFingerprint) && (
          <Note tone="ok">Paired with {current.serverName}. Its devices are under Devices.</Note>
        )}
        {(current?.state === 'denied' || current?.state === 'expired' || current?.state === 'failed') && <Note tone="error">Not paired: {current.message}</Note>}
        {(failures[cancelKey] || failures[confirmKey]) && <Note tone="error">Not done: {failures[cancelKey] ?? failures[confirmKey]}</Note>}
      </PanelBody>
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

/** Fingerprints compare by their hex digits alone; no digits is no fingerprint. */
const hex = (fingerprint: string | null | undefined) => hexOf(fingerprint) || undefined;
