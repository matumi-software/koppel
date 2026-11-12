import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Caption, Card, CopyValue, Identifier, LabeledValue, ModalDialog, Note, Row } from '@koppel/react-ui';
import type { ClientView } from '@/api/server';
import type { ServerKeysAction } from '@/pending/domains';
import { usePending } from '@/pending/pending-context';
import { serverKeysOptions } from '@/queries';

const names = (clients: ClientView[]) => clients.map((c) => c.name).join(', ');

interface ServerKeysProps {
  host: string;
  httpsPort?: number;
  clients: ClientView[];
  locked: boolean;
}

/**
 * Server role: the certificate clients pin, and what replaces it.
 * A new certificate is prepared first, so the paired clients learn its fingerprint, and switched to
 * after. Each action is pending until ServerKeysChanged brings the new state (see PendingProvider).
 */
export function ServerKeys({ host, httpsPort, clients, locked }: ServerKeysProps) {
  const keys = useQuery(serverKeysOptions());
  const { pending, failures, request } = usePending('serverKeys');
  const [confirmSwitch, setConfirmSwitch] = useState(false);

  const current = keys.data;
  if (!current) return null;

  const doing = (type: ServerKeysAction['type']) => type in pending;
  const busy = Object.keys(pending).length > 0;
  const missing = clients.filter((c) => !c.hasNextCertificate);
  const failure = Object.values(failures)[0];

  return (
    <Card label="Server certificate" gap="sm">
      <LabeledValue
        label="Server fingerprint"
        help={
          <>
            Clients connect to <Identifier>https://{host}{httpsPort ? `:${httpsPort}` : ''}</Identifier> and pin this fingerprint.
          </>
        }
      >
        <CopyValue value={current.fingerprint} wrap />
      </LabeledValue>

      {current.nextFingerprint ? (
        <>
          <LabeledValue
            label="New certificate"
            help={
              missing.length === 0
                ? 'Every paired client has its fingerprint, so switching is safe.'
                : `${clients.length - missing.length} of ${clients.length} clients have its fingerprint. Waiting for ${names(missing)}: a client gets it the next time it connects.`
            }
          >
            <CopyValue value={current.nextFingerprint} wrap />
          </LabeledValue>
          <Row>
            <Button variant="primary" isSending={doing('switch')} isDisabled={locked || busy} onPress={() => setConfirmSwitch(true)}>
              {doing('switch') ? 'Switching' : 'Switch to it'}
            </Button>
            <Button variant="secondary" isSending={doing('cancel')} isDisabled={locked || busy} onPress={() => request({ type: 'cancel' })}>
              {doing('cancel') ? 'Cancelling' : 'Cancel it'}
            </Button>
          </Row>
        </>
      ) : (
        <>
          <Caption tone="faint">To replace the certificate, prepare a new one first. Paired clients learn its fingerprint while this one stays in use, then you switch.</Caption>
          <Row>
            <Button variant="secondary" isSending={doing('prepare')} isDisabled={locked || busy} onPress={() => request({ type: 'prepare' })}>
              {doing('prepare') ? 'Preparing' : 'Prepare a new certificate'}
            </Button>
          </Row>
        </>
      )}

      {failure && <Note tone="error">Not done: {failure}</Note>}

      <ModalDialog isOpen={confirmSwitch} onOpenChange={setConfirmSwitch} title="Switch to the new certificate?">
        {missing.length === 0 ? (
          <Note>Every client has the new fingerprint and moves to it on its own. The old certificate is gone after this.</Note>
        ) : (
          <Note tone="error">
            {names(missing)} {missing.length === 1 ? 'has' : 'have'} not received the new fingerprint and will refuse this server after the switch. Pair{' '}
            {missing.length === 1 ? 'it' : 'them'} again, or enter the new fingerprint under Configuration on {missing.length === 1 ? 'that machine' : 'those machines'}.
          </Note>
        )}
        <Row justify="end">
          <Button variant="secondary" slot="close">
            Cancel
          </Button>
          <Button
            variant={missing.length === 0 ? 'primary' : 'danger'}
            isDisabled={locked}
            onPress={() => {
              setConfirmSwitch(false);
              request({ type: 'switch', nextFingerprint: current.nextFingerprint });
            }}
          >
            Switch
          </Button>
        </Row>
      </ModalDialog>
    </Card>
  );
}
