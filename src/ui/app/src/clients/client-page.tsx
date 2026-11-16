import { useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { RefreshCw, Trash2 } from 'lucide-react';
import { ActivityLog, Button, Callout, CopyValue, EmptyState, Facts, IconButton, PageHeader, SettingRow, SettingsList, StatusDot, Tile, TileStack } from '@heartwood/ui';
import { toEvents, useActivity } from '@/activity/activity-context';
import { getClientSecret, type ClientView } from '@/api/server';
import { states } from '@/brand';
import { serverKeysOptions } from '@/queries';
import { ConfirmDialog } from '@/shared/confirm-dialog';
import { useNow } from '@/shared/use-now';
import { useFrame } from '@/shell/frame-context';
import { addedOn, lastSeen, linkedWord } from './client-words';
import { useClients, useRevoke, useUsing } from './use-clients';
import { RotateSecretDialog } from './secret-dialogs';

export interface Back {
  label: string;
  onPress: () => void;
}

export function RevokeDialog({ client, onClose }: { client: Pick<ClientView, 'id' | 'name'>; onClose: () => void }) {
  const { locked } = useFrame();
  const { revoke } = useRevoke();
  return (
    <ConfirmDialog title={`Revoke ${client.name}?`} commit="Revoke" danger icon={Trash2} isDisabled={locked} onConfirm={() => revoke(client)} onClose={onClose}>
      {client.name} can no longer get tokens or attach devices. To let it back, link it again.
    </ConfirmDialog>
  );
}

/** Reads a client's kept secret only when asked, then shows it masked with its copy button. */
function KeptSecret({ client }: { client: ClientView }) {
  const show = useMutation({ mutationFn: () => getClientSecret(client.id) });
  if (show.data) return <CopyValue value={show.data} label="Client secret" masked revealable />;
  return (
    <Button variant="secondary" isPending={show.isPending} isDisabled={!client.hasSecret} onPress={() => show.mutate()}>
      Show
    </Button>
  );
}

/** One registered client as a page: what it uses, its facts, its secret, revoke, and its history. */
export function ClientPage({ clientId, back }: { clientId: string; back: Back }) {
  const { locked } = useFrame();
  const now = useNow();
  const client = useClients().data?.find((c) => c.id === clientId);
  const using = useUsing(clientId);
  const keys = useQuery(serverKeysOptions()).data;
  const { pending, failures } = useRevoke();
  const { items } = useActivity();
  const [revoking, setRevoking] = useState(false);
  const [rotating, setRotating] = useState(false);
  if (!client) {
    return (
      <div className="kp-page">
        <PageHeader back={back} title="Client not found" />
        <EmptyState title="This client is no longer registered">It was revoked, or the list has not loaded yet.</EmptyState>
      </div>
    );
  }
  const history = toEvents(items, client.id);
  const failure = failures[client.id];
  return (
    <div className="kp-page">
      <PageHeader
        back={back}
        title={client.name}
        description={
          pending[client.id] ? (
            <StatusDot tone="pending">Revoking</StatusDot>
          ) : using.length ? (
            `Using ${using.join(', ')}`
          ) : (
            `Last seen ${lastSeen(client, now).toLowerCase()}`
          )
        }
        actions={
          <>
            <IconButton icon={RefreshCw} label={`Rotate the secret of ${client.name}`} description="Its current secret stops working." variant="secondary" size="md" isDisabled={locked} onPress={() => setRotating(true)} />
            <IconButton icon={Trash2} label={`Revoke ${client.name}`} variant="secondary" size="md" danger isDisabled={locked || !!pending[client.id]} onPress={() => setRevoking(true)} />
          </>
        }
      />
      {failure ? (
        <div className="kp-text">
          <Callout tone="danger" title={`${client.name} is not revoked`}>
            {failure}
          </Callout>
        </div>
      ) : null}
      <TileStack>
        <Tile title="Details" padded>
          <Facts
            label={`Details of ${client.name}`}
            items={[
              { label: 'Client ID', value: client.id, copy: true },
              { label: 'Linked with', value: linkedWord(client) },
              { label: 'Added', value: addedOn(client) },
              { label: 'Last seen', value: lastSeen(client, now) },
              ...(keys?.nextFingerprint ? [{ label: 'New certificate', value: client.hasNextCertificate ? 'It has the fingerprint' : 'Not yet; it gets it when it next connects' }] : []),
            ]}
          />
        </Tile>
        <Tile title="Secret">
          <SettingsList label="Secret">
            <SettingRow
              title="Client secret"
              description={client.hasSecret ? 'Shown only when you ask for it.' : 'Added before Koppel kept secrets. Rotate it to get one you can copy.'}
              control={<KeptSecret key={`${client.id}-${client.hasSecret}`} client={client} />}
            />
          </SettingsList>
        </Tile>
        <ActivityLog states={states} events={history} meta={history.length ? String(history.length) : undefined} exportName={`${client.name}-history`} />
      </TileStack>
      {revoking ? <RevokeDialog client={client} onClose={() => setRevoking(false)} /> : null}
      {rotating ? <RotateSecretDialog client={client} onClose={() => setRotating(false)} /> : null}
    </div>
  );
}

