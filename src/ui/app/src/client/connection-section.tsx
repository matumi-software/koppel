import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Field, Panel } from '@koppel/react-ui';
import { testConnection, type ConnectionTestResult, type ConnectionView } from '@/api/client';
import { connectionKey as connectionPending } from '@/pending/domains';
import { usePending } from '@/pending/pending-context';
import { NearbyServers } from './nearby-servers';
import { connectionOptions } from '@/queries';

/**
 * Client role: the server this machine attaches devices from. Saving is pending until the pushed
 * connection (ConnectionChanged) or a refetch shows the address, fingerprint and client ID that were
 * sent; a push about some other change does not confirm it.
 */
export function ConnectionSection({ locked }: { locked: boolean }) {
  const connection = useQuery(connectionOptions());
  const { completed } = usePending('connection');
  if (!connection.data) {
    return null;
  }
  // Remount the form when a save is confirmed, so the secret that was typed is gone. A late confirm of
  // a save that had failed does not count, and a changed stored connection is taken in by the form
  // itself without dropping edits: the person may be typing by then.
  return (
    <>
      <NearbyServers locked={locked} />
      <ConnectionForm key={completed[connectionPending] ?? 0} stored={connection.data} locked={locked} />
    </>
  );
}

function ConnectionForm({ stored, locked }: { stored: ConnectionView; locked: boolean }) {
  const [serverUrl, setServerUrl] = useState(stored.serverUrl ?? '');
  const [fingerprint, setFingerprint] = useState(stored.serverFingerprint ?? '');
  const [clientId, setClientId] = useState(stored.clientId ?? '');
  const [secret, setSecret] = useState('');
  const [test, setTest] = useState<ConnectionTestResult>();
  const [testing, setTesting] = useState(false);
  const { pending, failures, request } = usePending('connection');
  const isPending = connectionPending in pending;
  const failure = failures[connectionPending];

  // When the stored connection changes, show it, unless the person has edits of their own to keep.
  const [seen, setSeen] = useState(stored);
  if (seen !== stored) {
    const edited =
      !isPending &&
      (serverUrl !== (seen.serverUrl ?? '') || fingerprint !== (seen.serverFingerprint ?? '') || clientId !== (seen.clientId ?? '') || secret !== '');
    setSeen(stored);
    if (!edited) {
      setServerUrl(stored.serverUrl ?? '');
      setFingerprint(stored.serverFingerprint ?? '');
      setClientId(stored.clientId ?? '');
      setSecret('');
    }
  }

  const save = () => {
    setTest(undefined);
    request({ serverUrl, serverFingerprint: fingerprint, clientId }, secret);
  };

  const runTest = () => {
    setTesting(true);
    setTest(undefined);
    testConnection()
      .then(setTest)
      .catch((error: unknown) => setTest({ ok: false, message: error instanceof Error ? error.message : 'The test could not run.' }))
      .finally(() => setTesting(false));
  };

  const fieldState = isPending ? 'pending' : undefined;
  const disabled = locked || isPending;
  const changed =
    serverUrl !== (stored.serverUrl ?? '') ||
    fingerprint !== (stored.serverFingerprint ?? '') ||
    clientId !== (stored.clientId ?? '') ||
    secret !== '';

  return (
    <Panel title="Server connection">
      <div className="kp-panel__body">
        <p className="kp-note">The Koppel server this machine attaches devices from. Pairing above fills this in; enter it by hand for a server on another network.</p>
        <Field label="Server address" mono placeholder="https://lab-rack-3:5240" value={serverUrl} onChange={setServerUrl} isDisabled={disabled} state={fieldState} />
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
        {failure && (
          <p className="app-error" role="status">
            Not saved: {failure}
          </p>
        )}
        <div className="app-inline">
          <Button variant="primary" isSending={isPending} isDisabled={locked || !changed} onPress={save}>
            Save connection
          </Button>
          <Button variant="secondary" isSending={testing} isDisabled={locked || isPending || changed || !stored.hasSecret} onPress={runTest}>
            Test connection
          </Button>
        </div>
        {test && (
          <p className={test.ok ? 'app-ok' : 'app-error'} role="status">
            {test.message}
          </p>
        )}
      </div>
    </Panel>
  );
}
