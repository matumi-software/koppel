import { useMutation, useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Field, Form, Note, Panel, Row, useAnnounce } from '@koppel/react-ui';
import { testConnection, type ConnectionView } from '@/api/client';
import { reasonOf } from '@/errors';
import { connectionKey } from '@/pending/domains';
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
      <ConnectionForm key={completed[connectionKey] ?? 0} stored={connection.data} locked={locked} />
    </>
  );
}

interface Draft {
  serverUrl: string;
  serverFingerprint: string;
  clientId: string;
  secret: string;
}

const draftOf = (stored: ConnectionView): Draft => ({
  serverUrl: stored.serverUrl ?? '',
  serverFingerprint: stored.serverFingerprint ?? '',
  clientId: stored.clientId ?? '',
  secret: '',
});

function ConnectionForm({ stored, locked }: { stored: ConnectionView; locked: boolean }) {
  // Only what the person changed is kept here. Everything else is the stored connection as it is now,
  // so a connection that changes underneath shows up in the fields nobody touched, and never
  // overwrites one that someone is typing in.
  const [edits, setEdits] = useState<Partial<Draft>>({});
  const base = draftOf(stored);
  const draft = { ...base, ...edits };
  const edit = (field: keyof Draft) => (value: string) =>
    setEdits((current) => {
      const rest = { ...current };
      delete rest[field];
      return value === base[field] ? rest : { ...rest, [field]: value };
    });

  const { pending, failures, request } = usePending('connection');
  const isPending = connectionKey in pending;
  const failure = failures[connectionKey];
  const announce = useAnnounce();
  const test = useMutation({
    mutationFn: testConnection,
    onSuccess: (result) => announce(`Connection test: ${result.message}`),
    onError: (error) => announce(`Connection test: ${reasonOf(error, 'The test could not run.')}`),
  });

  const save = () => {
    test.reset();
    request({ serverUrl: draft.serverUrl, serverFingerprint: draft.serverFingerprint, clientId: draft.clientId }, draft.secret);
  };

  const fieldState = isPending ? 'pending' : undefined;
  const disabled = locked || isPending;
  const changed = Object.keys(edits).length > 0;

  return (
    <Panel title="Server connection">
      <Form
        label="Server connection"
        variant="panel"
        onSubmit={(event) => {
          event.preventDefault();
          if (!locked && changed && !isPending) save();
        }}
      >
        <Note>The Koppel server this machine attaches devices from. Pairing above fills this in; enter it by hand for a server on another network.</Note>
        <Field label="Server address" name="serverUrl" mono placeholder="https://lab-rack-3:5240" value={draft.serverUrl} onChange={edit('serverUrl')} isDisabled={disabled} state={fieldState} />
        <Field
          label="Server fingerprint"
          name="serverFingerprint"
          mono
          placeholder="From the server's Clients section"
          value={draft.serverFingerprint}
          onChange={edit('serverFingerprint')}
          isDisabled={disabled}
          state={fieldState}
          help="Koppel only talks to the server whose certificate has this fingerprint."
        />
        <Field label="Client ID" name="clientId" mono value={draft.clientId} onChange={edit('clientId')} isDisabled={disabled} state={fieldState} />
        <Field
          label="Client secret"
          name="secret"
          type="password"
          autoComplete="off"
          placeholder={stored.hasSecret ? 'Stored, enter a new one to replace it' : ''}
          value={draft.secret}
          onChange={edit('secret')}
          isDisabled={disabled}
          state={fieldState}
          help="Encrypted on disk."
        />
        {failure && <Note tone="error">Not saved: {failure}</Note>}
        <Row>
          <Button variant="primary" type="submit" isSending={isPending} isDisabled={locked || !changed}>
            Save connection
          </Button>
          <Button variant="secondary" isSending={test.isPending} isDisabled={locked || isPending || changed || !stored.hasSecret} onPress={() => test.mutate()}>
            Test connection
          </Button>
        </Row>
        {test.isSuccess && <Note tone={test.data.ok ? 'ok' : 'error'}>{test.data.message}</Note>}
        {test.isError && <Note tone="error">{reasonOf(test.error, 'The test could not run.')}</Note>}
      </Form>
    </Panel>
  );
}
