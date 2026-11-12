import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Caption, Card, Field, Note, Row, SectionHeading } from '@koppel/react-ui';
import { pairingWindowMinutes, type PairingRequestView, type PairingWindowView } from '@/api/pairing';
import { usePending } from '@/pending/pending-context';
import { pairingRequestsOptions } from '@/queries';
import { pairingWindowKey } from '@/pending/domains';

/**
 * Server role: the pairing window and the clients asking to pair. A server takes pairing requests only
 * while a person has allowed it here, so a client that asks at any other time is turned away. A person types
 * the code the client shows. Allowing or closing pairing, and approving or refusing, are pending (see
 * PendingProvider) until PairingRequestsChanged brings the new window or leaves out the request; a wrong code
 * is refused at once.
 */
export function PairingRequests({ locked }: { locked: boolean }) {
  const requests = useQuery(pairingRequestsOptions());

  const list = requests.data?.requests ?? [];
  const exhausted = requests.data?.budgetExhausted ?? false;
  return (
    <>
      {requests.data && <PairingWindowCard window={requests.data.window} locked={locked} />}
      {(list.length > 0 || exhausted) && <PairingRequestList list={list} exhausted={exhausted} locked={locked} />}
    </>
  );
}

const untilText = (until: string | null) => (until ? new Date(until).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null);

function PairingWindowCard({ window, locked }: { window: PairingWindowView; locked: boolean }) {
  const { pending, failures, request } = usePending('pairingWindow');
  const sending = pending[pairingWindowKey]?.type;
  const failure = failures[pairingWindowKey];
  const until = untilText(window.until);
  return (
    <Card label="Pairing" gap="sm">
      <SectionHeading>Pairing</SectionHeading>
      {window.alwaysOpen ? (
        <Caption tone="faint">This server takes pairing requests at any time.</Caption>
      ) : window.open ? (
        <>
          <Caption tone="faint">
            {until ? `Pairing is open until ${until}. ` : 'Pairing is open. '}A client that asks now shows a code to type below.
          </Caption>
          <Row>
            <Button variant="secondary" isSending={sending === 'close'} isDisabled={locked || sending !== undefined} onPress={() => request({ type: 'close' })}>
              {sending === 'close' ? 'Closing' : 'Close pairing'}
            </Button>
            <Button variant="secondary" isSending={sending === 'open'} isDisabled={locked || sending !== undefined} onPress={() => request({ type: 'open' })}>
              {sending === 'open' ? 'Extending' : `Keep open for ${pairingWindowMinutes} more minutes`}
            </Button>
          </Row>
        </>
      ) : (
        <>
          <Caption tone="faint">
            This server takes no pairing requests until you allow it. Allow pairing while you are at the machine that needs to pair, then start pairing there.
          </Caption>
          <Row>
            <Button variant="primary" isSending={sending === 'open'} isDisabled={locked || sending !== undefined} onPress={() => request({ type: 'open' })}>
              {sending === 'open' ? 'Allowing' : `Allow pairing for ${pairingWindowMinutes} minutes`}
            </Button>
          </Row>
        </>
      )}
      {failure && <Note tone="error">Not done: {failure}</Note>}
    </Card>
  );
}

function PairingRequestList({ list, exhausted, locked }: { list: PairingRequestView[]; exhausted: boolean; locked: boolean }) {
  return (
    <Card label="Waiting to pair" gap="md">
      {exhausted && (
        <Note tone="error">
          This server has had too many pairing requests and is turning new ones away. It takes them again in a few minutes. If you did not expect this, something on your network is sending them.
        </Note>
      )}
      {list.length > 0 && <SectionHeading>{list.some((r) => r.state === 'waiting') ? 'Waiting to pair' : 'Pairing refused'}</SectionHeading>}
      {list.map((request) => (
        <PairingRequestRow key={request.id} request={request} locked={locked} />
      ))}
    </Card>
  );
}

function PairingRequestRow({ request, locked }: { request: PairingRequestView; locked: boolean }) {
  if (request.state === 'denied') {
    // Shown for a moment, so the request does not just vanish: the server dropped it after wrong codes.
    return (
      <Note tone="error">
        Refused {request.clientName} from {request.from}: {request.message ?? 'the request was refused.'}
      </Note>
    );
  }

  return <WaitingRow request={request} locked={locked} />;
}

function WaitingRow({ request, locked }: { request: PairingRequestView; locked: boolean }) {
  const [code, setCode] = useState('');
  const { pending: pendingRequests, failures, request: send } = usePending('pairingRequests');
  const pending = pendingRequests[request.id];
  const failure = failures[request.id];

  const until = new Date(request.expiresAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const digits = code.replace(/\D/g, '');
  return (
    <Field
      label={`${request.clientName} from ${request.from}`}
      mono
      placeholder="Code shown on the client"
      value={code}
      onChange={setCode}
      isDisabled={locked || pending !== undefined}
      state={pending ? 'pending' : failure ? 'invalid' : undefined}
      help={failure ? `Not done: ${failure}` : `Type the eight digits ${request.clientName} shows. The request stays open until ${until}.`}
      action={
        <Row as="span">
          <Button variant="primary" isSending={pending?.type === 'approve'} isDisabled={locked || pending !== undefined || digits.length !== 8} onPress={() => send({ type: 'approve', id: request.id, name: request.clientName, code: digits })}>
            Approve
          </Button>
          <Button variant="secondary" isSending={pending?.type === 'deny'} isDisabled={locked || pending !== undefined} onPress={() => send({ type: 'deny', id: request.id, name: request.clientName })}>
            Refuse
          </Button>
        </Row>
      }
    />
  );
}
