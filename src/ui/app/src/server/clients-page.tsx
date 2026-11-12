import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Caption, Card, EmptyNote, Glyph, Identifier, ItemList, ItemMain, Row, SectionHeading, StatusBadge } from '@koppel/react-ui';
import { since, useServerDevices } from '@/devices/use-devices';
import { useMachineName } from '@/shared/use-machine-name';
import { useNow } from '@/shared/use-now';
import { usePending } from '@/pending/pending-context';
import { Frame } from '@/shell/frame';
import { useFrame } from '@/shell/frame-context';
import { clientsOptions, identityOptions, serverKeysOptions } from '@/queries';
import { AddClient } from './add-client';
import { ClientDrawer } from './client-drawer';
import { seen } from './seen';
import { PairingRequests } from './pairing-requests';
import { ServerKeys } from './server-keys';

/**
 * Server role: the registered clients. Select one to see its ID and secret (Reveal and Copy), what it
 * has attached and its history; rotate or revoke it there. A revoke stays pending until
 * ClientsChanged arrives without that client.
 */
export function ClientsPage() {
  const host = useMachineName();
  const now = useNow();
  const { locked } = useFrame();
  const identity = useQuery(identityOptions());
  const clients = useQuery(clientsOptions());
  // While a new certificate is prepared, each client says whether it has its fingerprint.
  const handover = useQuery(serverKeysOptions()).data?.nextFingerprint != null;
  const devices = useServerDevices();
  const [selected, setSelected] = useState<string | null>(null);
  const { pending, failures, request } = usePending('clients');

  const attachedBy = (id: string) => (devices.data?.devices ?? []).filter((d) => d.state === 'attached' && d.attachedTo === id);
  const current = clients.data?.find((c) => c.id === selected);

  return (
    <Frame
      title="Clients"
      lede={`Machines that may attach devices from ${host}. Select one to copy its ID and secret.`}
      aside={
        current ? (
          <ClientDrawer
            key={current.id}
            client={current}
            attached={attachedBy(current.id).map((d) => `${d.name}${since(d.attachedSince)}`)}
            handover={handover}
            isRevoking={current.id in pending}
            revokeFailure={failures[current.id]}
            locked={locked}
            onRevoke={() => request({ id: current.id, name: current.name })}
            onClose={() => setSelected(null)}
          />
        ) : undefined
      }
      asideKey={current?.id}
    >
      <ServerKeys host={host} httpsPort={identity.data?.httpsPort} clients={clients.data ?? []} locked={locked} />

      <PairingRequests locked={locked} />

      <Card label="Registered clients" isFlush>
        {clients.data && clients.data.length === 0 && <EmptyNote>No clients yet. Add one below.</EmptyNote>}
        <ItemList
          label="Registered clients"
          items={clients.data ?? []}
          getKey={(c) => c.id}
          getText={(c) => c.name}
          selectedKey={selected}
          onSelectionChange={setSelected}
          isPending={(c) => c.id in pending}
        >
          {(client) => {
            const busy = client.id in pending;
            const using = attachedBy(client.id).length;
            return (
              <>
                <Glyph state={busy ? 'pending' : client.lastSeen ? 'live' : 'local'} />
                <ItemMain>
                  <SectionHeading level={3}>{client.name}</SectionHeading>
                  <Row>
                    <Identifier>{client.id}</Identifier>
                    <Caption tone={busy ? 'pending' : 'faint'}>
                      {busy ? 'Revoking… waiting for the server' : seen(client, now)}
                      {!busy && handover && (client.hasNextCertificate ? ' · Has the new certificate' : ' · Waiting for the new certificate')}
                    </Caption>
                  </Row>
                  {failures[client.id] && <Caption tone="fault">Not revoked: {failures[client.id]}</Caption>}
                </ItemMain>
                {using > 0 && <StatusBadge state="attached" detail={`${using} ${using === 1 ? 'device' : 'devices'}`} />}
              </>
            );
          }}
        </ItemList>
      </Card>

      <AddClient locked={locked} onAdded={setSelected} />
    </Frame>
  );
}
