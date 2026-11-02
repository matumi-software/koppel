import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Field } from '@koppel/react-ui';
import { approvePairing, denyPairing, getPairingRequests, type PairingRequest } from '@/api/pairing';
import { usePending } from '@/pending/pending-context';
import { pairingRequestsKey } from '@/query-keys';

/**
 * Server role: clients asking to pair. A person types the code the client shows. Approving or refusing is
 * pending (see PendingProvider) until PairingRequestsChanged arrives without the request; a wrong code is refused at once.
 */
export function PairingRequests({ locked }: { locked: boolean }) {
  const requests = useQuery({ queryKey: pairingRequestsKey, queryFn: getPairingRequests });

  const list = requests.data?.requests ?? [];
  const exhausted = requests.data?.budgetExhausted ?? false;
  if (list.length === 0 && !exhausted) {
    return null;
  }

  return (
    <section className="kp-card" aria-label="Waiting to pair" style={{ gap: 12 }}>
      {exhausted && (
        <p className="app-error" role="alert">
          This server has had too many pairing requests and is turning new ones away. It takes them again in a few minutes. If you did not expect this, something on your network is sending them.
        </p>
      )}
      {list.length > 0 && <span className="kp-type-heading">{list.some((r) => r.state === 'waiting') ? 'Waiting to pair' : 'Pairing refused'}</span>}
      {list.map((request) => (
        <PairingRequestRow key={request.id} request={request} locked={locked} />
      ))}
    </section>
  );
}

function PairingRequestRow({ request, locked }: { request: PairingRequest; locked: boolean }) {
  if (request.state === 'denied') {
    // Shown for a moment, so the request does not just vanish: the server dropped it after wrong codes.
    return (
      <p className="app-error" role="status">
        Refused {request.clientName} from {request.from}: {request.message ?? 'the request was refused.'}
      </p>
    );
  }

  return <WaitingRow request={request} locked={locked} />;
}

function WaitingRow({ request, locked }: { request: PairingRequest; locked: boolean }) {
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
      help={failure ? `Not done: ${failure}` : `Type the six digits ${request.clientName} shows. The request stays open until ${until}.`}
      action={
        <span className="app-inline">
          <Button variant="primary" isSending={pending === 'approve'} isDisabled={locked || pending !== undefined || digits.length !== 6} onPress={() => send(request.id, 'approve', () => approvePairing(request.id, digits))}>
            Approve
          </Button>
          <Button variant="secondary" isSending={pending === 'deny'} isDisabled={locked || pending !== undefined} onPress={() => send(request.id, 'deny', () => denyPairing(request.id))}>
            Refuse
          </Button>
        </span>
      }
    />
  );
}
