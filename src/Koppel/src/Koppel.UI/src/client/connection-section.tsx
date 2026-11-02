import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Field } from '@koppel/react-ui';
import { getConnection, putConnection, testConnection, type ConnectionTestResult, type ConnectionView } from '@/api/client';
import { useHubEvent } from '@/hub/hub-context';

const confirmTimeoutMs = 10_000;
const connectionKey = ['connection'] as const;

/**
 * Client role: the server this machine attaches devices from. Saving is pending until
 * ConnectionChanged arrives; the server writes connection.json once per accepted save and pushes
 * once, so the first push after a save confirms it.
 */
export function ConnectionSection({ locked }: { locked: boolean }) {
  const connection = useQuery({ queryKey: connectionKey, queryFn: getConnection });
  if (!connection.data) {
    return null;
  }
  // Remount the form when the confirmed connection changes, so it shows what is stored.
  return <ConnectionForm key={JSON.stringify(connection.data)} stored={connection.data} locked={locked} />;
}

function ConnectionForm({ stored, locked }: { stored: ConnectionView; locked: boolean }) {
  const queryClient = useQueryClient();
  const [serverUrl, setServerUrl] = useState(stored.serverUrl ?? '');
  const [fingerprint, setFingerprint] = useState(stored.serverFingerprint ?? '');
  const [clientId, setClientId] = useState(stored.clientId ?? '');
  const [secret, setSecret] = useState('');
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string>();
  const [test, setTest] = useState<ConnectionTestResult>();
  const [testing, setTesting] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const pendingRef = useRef(false);

  const settle = useCallback((reason?: string) => {
    clearTimeout(timer.current);
    pendingRef.current = false;
    setPending(false);
    setFailure(reason);
  }, []);

  const onConnectionChanged = useCallback(
    (view: ConnectionView) => {
      if (pendingRef.current) settle();
      queryClient.setQueryData(connectionKey, view);
    },
    [queryClient, settle],
  );
  useHubEvent('ConnectionChanged', onConnectionChanged);

  useEffect(() => () => clearTimeout(timer.current), []);

  const save = () => {
    pendingRef.current = true;
    setPending(true);
    setFailure(undefined);
    setTest(undefined);
    timer.current = setTimeout(() => settle('no confirmation from the server.'), confirmTimeoutMs);
    putConnection({ serverUrl, serverFingerprint: fingerprint, clientId, clientSecret: secret }).catch((error: unknown) =>
      settle(error instanceof Error ? error.message : 'the request failed.'),
    );
  };

  const runTest = () => {
    setTesting(true);
    testConnection()
      .then(setTest)
      .finally(() => setTesting(false));
  };

  const fieldState = pending ? 'pending' : undefined;
  const disabled = locked || pending;
  const changed =
    serverUrl !== (stored.serverUrl ?? '') ||
    fingerprint !== (stored.serverFingerprint ?? '') ||
    clientId !== (stored.clientId ?? '') ||
    secret !== '';

  return (
    <section className="app-section">
      <h2 className="kp-type-title app-section__title">Server connection</h2>
      <p className="app-note app-flush">The Koppel server this machine attaches devices from. Ask its admin for a client.</p>
      <Field label="Server address" mono placeholder="https://lab-rack-3:5241" value={serverUrl} onChange={setServerUrl} isDisabled={disabled} state={fieldState} />
      <Field
        label="Server fingerprint"
        mono
        placeholder="From the server's Clients section"
        value={fingerprint}
        onChange={setFingerprint}
        isDisabled={disabled}
        state={fieldState}
        help="Koppel only talks to the server whose certificate has this fingerprint."
      />
      <Field label="Client ID" mono value={clientId} onChange={setClientId} isDisabled={disabled} state={fieldState} />
      <Field
        label="Client secret"
        type="password"
        placeholder={stored.hasSecret ? 'Stored, enter a new one to replace it' : ''}
        value={secret}
        onChange={setSecret}
        isDisabled={disabled}
        state={fieldState}
        help="Encrypted on disk."
      />
      {failure && <p className="app-error">Not saved: {failure}</p>}
      <div className="app-inline">
        <Button variant="primary" isSending={pending} isDisabled={locked || !changed} onPress={save}>
          Save connection
        </Button>
        <Button variant="secondary" isSending={testing} isDisabled={locked || pending || changed || !stored.hasSecret} onPress={runTest}>
          Test connection
        </Button>
      </div>
      {test && (
        <p className={test.ok ? 'app-ok' : 'app-error'} role="status">
          {test.message}
        </p>
      )}
    </section>
  );
}
