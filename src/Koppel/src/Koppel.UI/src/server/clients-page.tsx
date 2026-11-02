import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useState } from 'react';
import { ActivityFeed, Button, CopyValue, Field, Glyph, ItemList, ModalDialog, SecretValue, StatusBadge } from '@koppel/react-ui';
import { toEntries, useActivity } from '@/activity/activity-context';
import { createClient, getClients, getClientSecret, getIdentity, revokeClient, rotateClientSecret, type ClientView, type CreatedClient } from '@/api/server';
import { since, useServerDevices } from '@/devices/use-devices';
import { useHubEvent } from '@/hub/hub-context';
import { useMachineName } from '@/shared/use-machine-name';
import { usePending } from '@/shared/use-pending';
import { Frame } from '@/shell/frame';
import { useFrame } from '@/shell/frame-context';
import { PairingRequests } from './pairing-requests';

const clientsKey = ['clients'] as const;
const dateFormat = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' });

const seen = (client: ClientView) => {
  if (!client.lastSeen) return 'Not seen since the server started';
  const minutes = Math.round((Date.now() - new Date(client.lastSeen).getTime()) / 60_000);
  return minutes < 1 ? 'Seen just now' : minutes < 60 ? `Seen ${minutes} min ago` : `Seen at ${new Date(client.lastSeen).toLocaleTimeString()}`;
};

/**
 * Server role: the registered clients. Select one to see its ID and secret (Reveal and Copy, like
 * Timebase Pulse), what it has attached and its history; rotate or revoke it there. A revoke stays
 * pending until ClientsChanged arrives without that client.
 */
export function ClientsPage() {
  const host = useMachineName();
  const { locked } = useFrame();
  const queryClient = useQueryClient();
  const identity = useQuery({ queryKey: ['identity'], queryFn: getIdentity });
  const clients = useQuery({ queryKey: clientsKey, queryFn: getClients });
  const devices = useServerDevices();
  const [selected, setSelected] = useState<string | null>(null);
  const describe = useCallback((id: string) => ({ text: `Revoke ${clients.data?.find((c) => c.id === id)?.name ?? id}`, subject: id }), [clients.data]);
  const { pending, failures, request, judge } = usePending<true>({ describe });

  const onClientsChanged = useCallback(
    (list: ClientView[]) => {
      queryClient.setQueryData(clientsKey, list);
      judge((id) => (list.some((c) => c.id === id) ? 'waiting' : 'confirmed'));
    },
    [queryClient, judge],
  );
  useHubEvent('ClientsChanged', onClientsChanged);

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
            isRevoking={current.id in pending}
            revokeFailure={failures[current.id]}
            locked={locked}
            onRevoke={() => request(current.id, true, () => revokeClient(current.id))}
            onClose={() => setSelected(null)}
          />
        ) : undefined
      }
      asideKey={current?.id}
    >
      {identity.data && (
        <section className="kp-card" aria-label="Server fingerprint" style={{ gap: 8 }}>
          <span className="kp-field__label">Server fingerprint</span>
          <CopyValue value={identity.data.fingerprint} wrap />
          <span className="kp-field__help">
            Clients connect to <code className="kp-id">https://{host}:{identity.data.httpsPort}</code> and pin this fingerprint.
          </span>
        </section>
      )}

      <PairingRequests locked={locked} />

      <section className="kp-card" aria-label="Registered clients" style={{ padding: 0, gap: 0 }}>
        {clients.data && clients.data.length === 0 && <p className="kp-mini__empty">No clients yet. Add one below.</p>}
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
                <div className="kp-list__main">
                  <span className="kp-type-heading">{client.name}</span>
                  <span className="kp-row">
                    <code className="kp-id">{client.id}</code>
                    <span className="kp-type-caption" style={{ color: busy ? 'var(--pending-text)' : 'var(--text-tertiary)' }}>
                      {busy ? 'Revoking… waiting for the server' : seen(client)}
                    </span>
                  </span>
                  {failures[client.id] && <span className="kp-error kp-type-caption">Not revoked: {failures[client.id]}</span>}
                </div>
                {using > 0 && <StatusBadge state="attached" detail={`${using} ${using === 1 ? 'device' : 'devices'}`} />}
              </>
            );
          }}
        </ItemList>
      </section>

      <AddClient locked={locked} onAdded={setSelected} />
    </Frame>
  );
}

interface ClientDrawerProps {
  client: ClientView;
  attached: string[];
  isRevoking: boolean;
  revokeFailure?: string;
  locked: boolean;
  onRevoke: () => void;
  onClose: () => void;
}

function ClientDrawer({ client, attached, isRevoking, revokeFailure, locked, onRevoke, onClose }: ClientDrawerProps) {
  const { items } = useActivity();
  const [confirmRotate, setConfirmRotate] = useState(false);
  const [confirmRevoke, setConfirmRevoke] = useState(false);
  const [rotated, setRotated] = useState<string>();
  const [rotating, setRotating] = useState(false);
  const [rotateError, setRotateError] = useState<string>();

  const rotate = () => {
    setRotating(true);
    setRotateError(undefined);
    rotateClientSecret(client.id)
      .then((secret) => {
        setRotated(secret);
        setConfirmRotate(false);
      })
      .catch((e: unknown) => setRotateError(e instanceof Error ? e.message : 'the request failed.'))
      .finally(() => setRotating(false));
  };

  return (
    <>
      <div className="kp-drawer__head">
        <div style={{ display: 'grid', gap: 4, minWidth: 0 }}>
          <h2>{client.name}</h2>
          <span className="kp-type-caption" style={{ color: 'var(--text-tertiary)' }}>
            Added {dateFormat.format(new Date(client.createdAt))} · {seen(client)}
          </span>
        </div>
        <Button variant="ghost" aria-label="Close details" onPress={onClose}>
          Close
        </Button>
      </div>

      <div className="kp-field">
        <span className="kp-field__label">Client ID</span>
        <CopyValue value={client.id} />
      </div>
      <div className="kp-field">
        <span className="kp-field__label">Client secret</span>
        {rotated ? (
          <>
            <CopyValue value={rotated} wrap />
            <span className="kp-field__help">New secret. Put it on {client.name} now; the old one no longer works.</span>
          </>
        ) : (
          <SecretValue
            key={client.hasSecret ? 'kept' : 'missing'}
            loadSecret={() => getClientSecret(client.id)}
            unavailable={client.hasSecret ? undefined : 'This client was added before Koppel kept secrets. Rotate the secret to get one you can copy.'}
          />
        )}
      </div>

      <div className="kp-field">
        <span className="kp-field__label">Using</span>
        {attached.length ? (
          <div className="kp-chips">
            {attached.map((name) => (
              <span key={name} className="kp-chip">
                {name}
              </span>
            ))}
          </div>
        ) : (
          <span className="kp-field__help">No devices attached right now.</span>
        )}
      </div>

      <div className="kp-row">
        <Button variant="secondary" isDisabled={locked} onPress={() => setConfirmRotate(true)}>
          Rotate secret
        </Button>
        <Button variant="danger" isSending={isRevoking} isDisabled={locked} onPress={() => setConfirmRevoke(true)}>
          {isRevoking ? 'Revoking' : 'Revoke'}
        </Button>
      </div>
      {revokeFailure && <p className="kp-error">Not revoked: {revokeFailure}</p>}

      <hr style={{ border: 0, height: 1, background: 'var(--border-default)', margin: 0, width: '100%' }} />
      <h3 className="kp-type-heading" style={{ margin: 0 }}>
        History
      </h3>
      <ActivityFeed entries={toEntries(items, client.id)} empty="Nothing about this client yet." />

      <ModalDialog isOpen={confirmRotate} onOpenChange={setConfirmRotate} title={`Rotate the secret for ${client.name}?`}>
        <p className="kp-note">{client.name} stops getting new tokens until you give it the new secret. Devices it has attached stay attached until it reconnects.</p>
        {rotateError && <p className="kp-error">Not rotated: {rotateError}</p>}
        <div className="kp-row" style={{ justifyContent: 'flex-end' }}>
          <Button variant="secondary" slot="close">
            Cancel
          </Button>
          <Button variant="primary" isSending={rotating} onPress={rotate}>
            Rotate secret
          </Button>
        </div>
      </ModalDialog>

      <ModalDialog isOpen={confirmRevoke} onOpenChange={setConfirmRevoke} title={`Revoke ${client.name}?`}>
        <p className="kp-note">{client.name} can no longer get tokens or attach devices. To let it back, add it again and give it the new ID and secret.</p>
        <div className="kp-row" style={{ justifyContent: 'flex-end' }}>
          <Button variant="secondary" slot="close">
            Cancel
          </Button>
          <Button
            variant="danger"
            onPress={() => {
              setConfirmRevoke(false);
              onRevoke();
            }}
          >
            Revoke
          </Button>
        </div>
      </ModalDialog>
    </>
  );
}

function AddClient({ locked, onAdded }: { locked: boolean; onAdded: (id: string) => void }) {
  const [name, setName] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string>();
  const [created, setCreated] = useState<CreatedClient>();

  const submit = () => {
    setSending(true);
    setError(undefined);
    createClient(name)
      .then((client) => {
        setCreated(client);
        setName('');
        onAdded(client.clientId);
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : 'the request failed.'))
      .finally(() => setSending(false));
  };

  return (
    <section className="kp-card" aria-label="Add a client" style={{ gap: 8 }}>
      <Field
        label="New client"
        placeholder="Machine name, such as bench-2"
        value={name}
        onChange={setName}
        isDisabled={locked || sending}
        state={error ? 'invalid' : undefined}
        help={error ? `Not added: ${error}` : 'You get a client ID and a secret. You can copy both again later from the client.'}
        action={
          <Button variant="primary" isSending={sending} isDisabled={locked || name.trim() === ''} onPress={submit}>
            Add client
          </Button>
        }
      />
      <ModalDialog isOpen={!!created} onOpenChange={(open) => !open && setCreated(undefined)} title={`${created?.name ?? 'Client'} is added`}>
        {created && (
          <>
            <p className="kp-note">Put the ID and secret on {created.name}, under Configuration, Server connection, with the server fingerprint.</p>
            <div className="kp-field">
              <span className="kp-field__label">Client ID</span>
              <CopyValue value={created.clientId} />
            </div>
            <div className="kp-field">
              <span className="kp-field__label">Client secret</span>
              <CopyValue value={created.clientSecret} wrap />
            </div>
            <div className="kp-row" style={{ justifyContent: 'flex-end' }}>
              <Button variant="primary" slot="close">
                Done
              </Button>
            </div>
          </>
        )}
      </ModalDialog>
    </section>
  );
}
