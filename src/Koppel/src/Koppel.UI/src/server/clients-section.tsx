import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Field } from '@koppel/react-ui';
import { createClient, getClients, getIdentity, revokeClient, type ClientView, type CreatedClient } from '@/api/server';
import { useHubEvent } from '@/hub/hub-context';
import { CopyButton } from '@/shared/copy-button';

const confirmTimeoutMs = 10_000;
const clientsKey = ['clients'] as const;

/**
 * Server role: the fingerprint clients pin, and the registered clients. A revoke stays pending until
 * ClientsChanged arrives without that client; a new client appears when ClientsChanged brings it.
 */
export function ClientsSection({ locked }: { locked: boolean }) {
  const queryClient = useQueryClient();
  const identity = useQuery({ queryKey: ['identity'], queryFn: getIdentity });
  const clients = useQuery({ queryKey: clientsKey, queryFn: getClients });
  const [revoking, setRevoking] = useState<ReadonlySet<string>>(new Set());
  const [failure, setFailure] = useState<string>();
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  const settle = useCallback((id: string, reason?: string) => {
    clearTimeout(timers.current.get(id));
    timers.current.delete(id);
    setRevoking((current) => new Set([...current].filter((x) => x !== id)));
    setFailure(reason);
  }, []);

  const onClientsChanged = useCallback(
    (list: ClientView[]) => {
      queryClient.setQueryData(clientsKey, list);
      for (const id of timers.current.keys()) {
        if (!list.some((c) => c.id === id)) settle(id);
      }
    },
    [queryClient, settle],
  );
  useHubEvent('ClientsChanged', onClientsChanged);

  useEffect(() => {
    const active = timers.current;
    return () => active.forEach(clearTimeout);
  }, []);

  const revoke = (id: string) => {
    if (revoking.has(id)) return;
    setRevoking((current) => new Set(current).add(id));
    setFailure(undefined);
    timers.current.set(id, setTimeout(() => settle(id, 'no confirmation from the server.'), confirmTimeoutMs));
    revokeClient(id).catch((error: unknown) => settle(id, error instanceof Error ? error.message : 'the request failed.'));
  };

  return (
    <section className="app-section">
      <h2 className="kp-type-title app-section__title">Clients</h2>
      <p className="app-note app-flush">Machines that may attach devices from this server. Each gets its own ID and secret.</p>

      {identity.data && (
        <div className="app-kv">
          <span className="kp-field__label">Server fingerprint</span>
          <div className="app-inline">
            <code className="kp-id app-wrap">{identity.data.fingerprint}</code>
            <CopyButton text={identity.data.fingerprint} />
          </div>
          <span className="kp-field__help">
            Clients connect to <code className="kp-id">https://{window.location.hostname}:{identity.data.httpsPort}</code> and pin this fingerprint.
          </span>
        </div>
      )}

      {clients.data && clients.data.length === 0 && <p className="app-note app-flush">No clients yet.</p>}
      {clients.data && clients.data.length > 0 && (
        <ul className="app-list">
          {clients.data.map((client) => (
            <li key={client.id} className="app-list__row">
              <div className="app-list__main">
                <span className="kp-type-heading">{client.name}</span>
                <span className="app-inline">
                  <code className="kp-id">{client.id}</code>
                  <span className="kp-field__help">Added {new Date(client.createdAt).toLocaleString()}</span>
                </span>
              </div>
              <Button variant="danger" isSending={revoking.has(client.id)} isDisabled={locked} onPress={() => revoke(client.id)}>
                {revoking.has(client.id) ? 'Revoking' : 'Revoke'}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {failure && <p className="app-error">Not revoked: {failure}</p>}

      <AddClient locked={locked} />
    </section>
  );
}

function AddClient({ locked }: { locked: boolean }) {
  const [name, setName] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string>();
  const [created, setCreated] = useState<CreatedClient>();

  const add = () => {
    setSending(true);
    setError(undefined);
    createClient(name)
      .then((client) => {
        setCreated(client);
        setName('');
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : 'the request failed.'))
      .finally(() => setSending(false));
  };

  if (created) {
    return (
      <div className="app-callout" role="status">
        <p className="app-flush">
          <strong>{created.name}</strong> is added. Copy the ID and secret into that machine now; the secret is not shown again.
        </p>
        <div className="app-kv">
          <span className="kp-field__label">Client ID</span>
          <div className="app-inline">
            <code className="kp-id">{created.clientId}</code>
            <CopyButton text={created.clientId} />
          </div>
        </div>
        <div className="app-kv">
          <span className="kp-field__label">Client secret</span>
          <div className="app-inline">
            <code className="kp-id app-wrap">{created.clientSecret}</code>
            <CopyButton text={created.clientSecret} />
          </div>
        </div>
        <div>
          <Button variant="secondary" onPress={() => setCreated(undefined)}>
            Done
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="app-row">
      <Field
        label="New client"
        placeholder="Machine name, such as bench-2"
        value={name}
        onChange={setName}
        isDisabled={locked || sending}
        state={error ? 'invalid' : undefined}
        help={error ? `Not added: ${error}` : undefined}
        action={
          <Button variant="primary" isSending={sending} isDisabled={locked || name.trim() === ''} onPress={add}>
            Add client
          </Button>
        }
      />
    </div>
  );
}
