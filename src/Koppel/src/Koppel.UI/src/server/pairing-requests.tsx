import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Field } from '@koppel/react-ui';
import { approvePairing, denyPairing, getPairingRequests, type PairingRequest } from '@/api/pairing';
import { useHubEvent } from '@/hub/hub-context';

const requestsKey = ['pairing-requests'] as const;
const confirmTimeoutMs = 10_000;

/**
 * Server role: clients asking to pair. A person types the code the client shows. Approving is
 * pending until PairingRequestsChanged arrives without the request; a wrong code is refused at once.
 */
export function PairingRequests({ locked }: { locked: boolean }) {
  const queryClient = useQueryClient();
  const requests = useQuery({ queryKey: requestsKey, queryFn: getPairingRequests });
  const onRequests = useCallback((list: PairingRequest[]) => queryClient.setQueryData(requestsKey, list), [queryClient]);
  useHubEvent('PairingRequestsChanged', onRequests);

  if (!requests.data || requests.data.length === 0) {
    return null;
  }

  return (
    <section className="kp-card" aria-label="Waiting to pair" style={{ gap: 12 }}>
      <span className="kp-type-heading">Waiting to pair</span>
      {requests.data.map((request) => (
        <PairingRequestRow key={request.id} request={request} locked={locked} />
      ))}
    </section>
  );
}

function PairingRequestRow({ request, locked }: { request: PairingRequest; locked: boolean }) {
  const [code, setCode] = useState('');
  const [pending, setPending] = useState<'approve' | 'deny'>();
  const [failure, setFailure] = useState<string>();
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  // The row unmounts when the confirming push no longer lists the request, which ends the pending state.
  const send = (kind: 'approve' | 'deny', change: () => Promise<void>) => {
    setPending(kind);
    setFailure(undefined);
    timer.current = setTimeout(() => {
      setPending(undefined);
      setFailure('no confirmation from the server.');
    }, confirmTimeoutMs);
    change().catch((error: unknown) => {
      clearTimeout(timer.current);
      setPending(undefined);
      setFailure(error instanceof Error ? error.message : 'the request failed.');
    });
  };

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
      help={failure ? `Not approved: ${failure}` : `Type the six digits ${request.clientName} shows. The request stays open until ${until}.`}
      action={
        <span className="app-inline">
          <Button variant="primary" isSending={pending === 'approve'} isDisabled={locked || pending !== undefined || digits.length !== 6} onPress={() => send('approve', () => approvePairing(request.id, digits))}>
            Approve
          </Button>
          <Button variant="secondary" isSending={pending === 'deny'} isDisabled={locked || pending !== undefined} onPress={() => send('deny', () => denyPairing(request.id))}>
            Refuse
          </Button>
        </span>
      }
    />
  );
}
