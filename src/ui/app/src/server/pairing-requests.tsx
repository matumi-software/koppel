import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Field } from '@koppel/react-ui';
import { approvePairing, closePairingWindow, denyPairing, getPairingRequests, openPairingWindow, pairingWindowMinutes, type PairingRequest, type PairingWindow } from '@/api/pairing';
import { usePending } from '@/pending/pending-context';
import { pairingRequestsKey } from '@/query-keys';

/**
 * Server role: the pairing window and the clients asking to pair. A server takes pairing requests only
 * while a person has allowed it here, so a client that asks at any other time is turned away. A person types
 * the code the client shows. Allowing or closing pairing, and approving or refusing, are pending (see
 * PendingProvider) until PairingRequestsChanged brings the new window or leaves out the request; a wrong code
 * is refused at once.
 */
export function PairingRequests({ locked }: { locked: boolean }) {
  const requests = useQuery({ queryKey: pairingRequestsKey, queryFn: getPairingRequests });

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

function PairingWindowCard({ window, locked }: { window: PairingWindow; locked: boolean }) {
  const { pending, failures, request } = usePending('pairingWindow');
  const sending = pending.window;
  const failure = failures.window;
  const until = untilText(window.until);
  return (
    <section className="kp-card" aria-label="Pairing" style={{ gap: 8 }}>
      <span className="kp-type-heading">Pairing</span>
      {window.alwaysOpen ? (
        <span className="kp-field__help" role="status">
          This server takes pairing requests at any time.
        </span>
      ) : window.open ? (
        <>
          <span className="kp-field__help" role="status">
            {until ? `Pairing is open until ${until}. ` : 'Pairing is open. '}A client that asks now shows a code to type below.
          </span>
          <div className="kp-row">
            <Button variant="secondary" isSending={sending === 'close'} isDisabled={locked || sending !== undefined} onPress={() => request('window', 'close', closePairingWindow)}>
              {sending === 'close' ? 'Closing' : 'Close pairing'}
            </Button>
            <Button variant="secondary" isSending={sending === 'open'} isDisabled={locked || sending !== undefined} onPress={() => request('window', 'open', openPairingWindow)}>
              {sending === 'open' ? 'Extending' : `Keep open for ${pairingWindowMinutes} more minutes`}
            </Button>
          </div>
        </>
      ) : (
        <>
          <span className="kp-field__help" role="status">
            This server takes no pairing requests until you allow it. Allow pairing while you are at the machine that needs to pair, then start pairing there.
          </span>
          <div className="kp-row">
            <Button variant="primary" isSending={sending === 'open'} isDisabled={locked || sending !== undefined} onPress={() => request('window', 'open', openPairingWindow)}>
              {sending === 'open' ? 'Allowing' : `Allow pairing for ${pairingWindowMinutes} minutes`}
            </Button>
          </div>
        </>
      )}
      {failure && <p className="kp-error">Not done: {failure}</p>}
    </section>
  );
}

function PairingRequestList({ list, exhausted, locked }: { list: PairingRequest[]; exhausted: boolean; locked: boolean }) {
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
      help={failure ? `Not done: ${failure}` : `Type the eight digits ${request.clientName} shows. The request stays open until ${until}.`}
      action={
        <span className="app-inline">
          <Button variant="primary" isSending={pending === 'approve'} isDisabled={locked || pending !== undefined || digits.length !== 8} onPress={() => send(request.id, 'approve', () => approvePairing(request.id, digits))}>
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
