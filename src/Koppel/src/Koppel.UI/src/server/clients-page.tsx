import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useState } from 'react';
import { ActivityFeed, Button, Field, Glyph, ModalDialog } from '@koppel/react-ui';
import { toEntries, useActivity } from '@/activity/activity-context';
import { createClient, getClients, getIdentity, revokeClient, type ClientView, type CreatedClient } from '@/api/server';
import { useHubEvent } from '@/hub/hub-context';
import { CopyButton } from '@/shared/copy-button';
import { usePending } from '@/shared/use-pending';
import { Frame } from '@/shell/frame';
import { useMachineName } from '@/shared/use-machine-name';
import { useFrame } from '@/shell/frame-context';

const clientsKey = ['clients'] as const;
/**
 * Server role: the fingerprint clients pin, and the registered clients. A revoke stays pending until
 * ClientsChanged arrives without that client; a new client shows its one-time secret in a dialog.
 */
export function ClientsPage() {
  const host = useMachineName();
  const { locked } = useFrame();
  const { items } = useActivity();
  const queryClient = useQueryClient();
  const identity = useQuery({ queryKey: ['identity'], queryFn: getIdentity });
  const clients = useQuery({ queryKey: clientsKey, queryFn: getClients });
  const describe = useCallback((id: string) => ({ text: `Revoke ${clients.data?.find((c) => c.id === id)?.name ?? id}` }), [clients.data]);
  const { pending, failures, request, judge } = usePending<true>({ describe });

  const onClientsChanged = useCallback(
    (list: ClientView[]) => {
      queryClient.setQueryData(clientsKey, list);
      judge((id) => (list.some((c) => c.id === id) ? 'waiting' : 'confirmed'));
    },
    [queryClient, judge],
  );
  useHubEvent('ClientsChanged', onClientsChanged);

  return (
    <Frame
      title="Clients"
      lede={`Machines that may attach devices from ${host}. Each gets its own ID and secret.`}
      aside={
        <>
          <h2 className="kp-type-heading" style={{ margin: 0 }}>
            Activity
          </h2>
          <ActivityFeed entries={toEntries(items)} empty="Changes show here when the server confirms or rejects them." />
        </>
      }
    >
      {identity.data && (
        <section className="kp-card" aria-label="Server fingerprint" style={{ gap: 8 }}>
          <span className="kp-field__label">Server fingerprint</span>
          <div className="kp-row">
            <code className="kp-id" style={{ whiteSpace: 'normal', overflowWrap: 'anywhere' }}>
              {identity.data.fingerprint}
            </code>
            <CopyButton text={identity.data.fingerprint} />
          </div>
          <span className="kp-field__help">
            Clients connect to <code className="kp-id">https://{host}:{identity.data.httpsPort}</code> and pin this fingerprint.
          </span>
        </section>
      )}

      <section className="kp-card" aria-label="Registered clients" style={{ padding: 0, gap: 0 }}>
        {clients.data && clients.data.length === 0 && <p className="kp-mini__empty">No clients yet. Add one below.</p>}
        <ul className="kp-list">
          {(clients.data ?? []).map((client) => {
            const busy = client.id in pending;
            return (
              <li key={client.id} data-state={busy ? 'pending' : undefined}>
                <Glyph state={busy ? 'pending' : 'live'} />
                <div className="kp-list__main">
                  <span className="kp-type-heading">{client.name}</span>
                  <span className="kp-row">
                    <code className="kp-id">{client.id}</code>
                    <span className="kp-type-caption" style={{ color: busy ? 'var(--pending-text)' : 'var(--text-tertiary)' }}>
                      {busy ? 'Revoking… waiting for the server' : `Added ${new Date(client.createdAt).toLocaleDateString()}`}
                    </span>
                  </span>
                  {failures[client.id] && <span className="kp-error kp-type-caption">Not revoked: {failures[client.id]}</span>}
                </div>
                <Button variant="danger" isSending={busy} isDisabled={locked} onPress={() => request(client.id, true, () => revokeClient(client.id))}>
                  {busy ? 'Revoking' : 'Revoke'}
                </Button>
              </li>
            );
          })}
        </ul>
      </section>

      <AddClient locked={locked} />
    </Frame>
  );
}

function AddClient({ locked }: { locked: boolean }) {
  const { add } = useActivity();
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
        add({ state: 'shared', lead: 'Done.', text: `Client ${client.name} added` });
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
        help={error ? `Not added: ${error}` : 'You get a client ID and a secret. The secret is shown once.'}
        action={
          <Button variant="primary" isSending={sending} isDisabled={locked || name.trim() === ''} onPress={submit}>
            Add client
          </Button>
        }
      />
      <ModalDialog isOpen={!!created} onOpenChange={(open) => !open && setCreated(undefined)} title={`${created?.name ?? 'Client'} is added`}>
        {created && (
          <>
            <p className="kp-note">Copy the ID and secret into {created.name} now. The secret is not shown again.</p>
            <div className="kp-field">
              <span className="kp-field__label">Client ID</span>
              <div className="kp-row">
                <code className="kp-id">{created.clientId}</code>
                <CopyButton text={created.clientId} />
              </div>
            </div>
            <div className="kp-field">
              <span className="kp-field__label">Client secret</span>
              <div className="kp-row">
                <code className="kp-id" style={{ whiteSpace: 'normal', overflowWrap: 'anywhere' }}>
                  {created.clientSecret}
                </code>
                <CopyButton text={created.clientSecret} />
              </div>
              <span className="kp-field__help">On {created.name}, open Server connection and paste both with the server fingerprint.</span>
            </div>
            <div className="kp-row" style={{ justifyContent: 'flex-end' }}>
              <Button variant="primary" slot="close">
                I have copied them
              </Button>
            </div>
          </>
        )}
      </ModalDialog>
    </section>
  );
}
