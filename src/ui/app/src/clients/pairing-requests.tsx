import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { X } from 'lucide-react';
import { Button, Callout, IconButton, SettingRow, SettingsList, Switch, TextField, Tile } from '@heartwood/ui';
import { pairingWindowMinutes, type PairingRequestView } from '@/api/pairing';
import { pairingWindowKey } from '@/pending/domains';
import { usePending } from '@/pending/pending-context';
import { pairingRequestsOptions } from '@/queries';
import { clock } from '@/shared/time';
import { useFrame } from '@/shell/frame-context';


/**
 * The setting row that lets clients ask to pair: a switch for the window, pending until
 * PairingRequestsChanged says it is open or closed. Turning it on keeps it open for a few minutes.
 */
export function PairingWindowRow() {
  const { locked } = useFrame();
  const overview = useQuery(pairingRequestsOptions()).data;
  const { pending, failures, request } = usePending('pairingWindow');
  if (!overview) return null;
  const { window } = overview;
  const action = pending[pairingWindowKey];
  const on = action ? action.type === 'open' : window.open || window.alwaysOpen;
  const failure = failures[pairingWindowKey];
  return (
    <SettingRow
      title="Pairing requests"
      description={
        failure
          ? `Not done: ${failure}`
          : window.alwaysOpen
            ? 'This server takes pairing requests at any time.'
            : window.open && window.until
              ? `Open until ${clock.format(new Date(window.until))}. A computer that asks shows a code to type here.`
              : `Lets a computer ask to pair, for ${pairingWindowMinutes} minutes. You type the code it shows.`
      }
      control={<Switch aria-label="Pairing requests" isSelected={on} isPending={!!action} isDisabled={locked || window.alwaysOpen} onChange={(open) => request({ type: open ? 'open' : 'close' })} />}
    />
  );
}

/** The computers asking to pair: type the eight digits each shows to approve, or refuse. Nothing shows when none ask. */
export function PairingRequestsTile() {
  const overview = useQuery(pairingRequestsOptions()).data;
  if (!overview || (overview.requests.length === 0 && !overview.budgetExhausted)) return null;
  const waiting = overview.requests.filter((r) => r.state === 'waiting');
  const denied = overview.requests.filter((r) => r.state === 'denied');
  return (
    <Tile title="Waiting to pair" meta={waiting.length ? String(waiting.length) : undefined} padded>
      <div className="kp-stack-3">
        {overview.budgetExhausted ? (
          <Callout tone="warning" title="New requests are being turned away">
            This server had too many pairing requests and takes them again in a few minutes. If you did not expect this, something on your network is sending them.
          </Callout>
        ) : null}
        {denied.map((r) => (
          <Callout key={r.id} tone="danger" title={`Refused ${r.clientName} from ${r.from}`}>
            {r.message ?? 'The request was refused.'}
          </Callout>
        ))}
        {waiting.length ? (
          <SettingsList label="Computers waiting to pair">
            {waiting.map((r) => (
              <WaitingRow key={r.id} request={r} />
            ))}
          </SettingsList>
        ) : null}
      </div>
    </Tile>
  );
}

function WaitingRow({ request }: { request: PairingRequestView }) {
  const { locked } = useFrame();
  const [code, setCode] = useState('');
  const { pending, failures, request: send } = usePending('pairingRequests');
  const action = pending[request.id];
  const failure = failures[request.id];
  const digits = code.replace(/\D/g, '');
  return (
    <SettingRow
      title={`${request.clientName} from ${request.from}`}
      description={failure ? `Not done: ${failure}` : `Type the eight digits it shows. The request stays open until ${clock.format(new Date(request.expiresAt))}.`}
      control={
        <span className="kp-line">
          <TextField ariaLabel={`Code shown on ${request.clientName}`} placeholder="8 digits" mono inputMode="numeric" value={code} onChange={setCode} isDisabled={locked || !!action} isInvalid={!!failure} width="var(--size-width-xs)" />
          <Button variant="primary" isPending={action?.type === 'approve'} isDisabled={locked || !!action || digits.length !== 8} onPress={() => send({ type: 'approve', id: request.id, name: request.clientName, code: digits })}>
            Approve
          </Button>
          <IconButton icon={X} label={`Refuse ${request.clientName}`} variant="secondary" isDisabled={locked || !!action} onPress={() => send({ type: 'deny', id: request.id, name: request.clientName })} />
        </span>
      }
    />
  );
}
