import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, CopyValue, ModalDialog } from '@koppel/react-ui';
import { cancelCertificate, getServerKeys, prepareCertificate, rotateSigningKey, switchCertificate, type ClientView } from '@/api/server';
import { usePending } from '@/pending/pending-context';
import { serverKeysKey } from '@/query-keys';

const dateFormat = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' });

const names = (clients: ClientView[]) => clients.map((c) => c.name).join(', ');

interface ServerKeysProps {
  host: string;
  httpsPort?: number;
  clients: ClientView[];
  locked: boolean;
}

/**
 * Server role: the certificate clients pin and the key tokens are signed with, and what replaces them.
 * A new certificate is prepared first, so the paired clients learn its fingerprint, and switched to
 * after. Each action is pending until ServerKeysChanged brings the new state (see PendingProvider).
 */
export function ServerKeys({ host, httpsPort, clients, locked }: ServerKeysProps) {
  const keys = useQuery({ queryKey: serverKeysKey, queryFn: getServerKeys });
  const { pending, failures, request } = usePending('serverKeys');
  const [confirmSwitch, setConfirmSwitch] = useState(false);

  const current = keys.data;
  if (!current) return null;

  const busy = Object.keys(pending).length > 0;
  const missing = clients.filter((c) => !c.hasNextCertificate);
  const failure = Object.values(failures)[0];
  const retiredUntil = current.retiredKeysUntil ? new Date(current.retiredKeysUntil).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null;

  return (
    <section className="kp-card" aria-label="Server certificate" style={{ gap: 8 }}>
      <span className="kp-field__label">Server fingerprint</span>
      <CopyValue value={current.fingerprint} wrap />
      <span className="kp-field__help">
        Clients connect to <code className="kp-id">https://{host}{httpsPort ? `:${httpsPort}` : ''}</code> and pin this fingerprint.
      </span>

      {current.nextFingerprint ? (
        <>
          <span className="kp-field__label">New certificate</span>
          <CopyValue value={current.nextFingerprint} wrap />
          <span className="kp-field__help" role="status">
            {missing.length === 0
              ? 'Every paired client has its fingerprint, so switching is safe.'
              : `${clients.length - missing.length} of ${clients.length} clients have its fingerprint. Waiting for ${names(missing)}: a client gets it the next time it connects.`}
          </span>
          <div className="kp-row">
            <Button variant="primary" isSending={'switch' in pending} isDisabled={locked || busy} onPress={() => setConfirmSwitch(true)}>
              {'switch' in pending ? 'Switching' : 'Switch to it'}
            </Button>
            <Button variant="secondary" isSending={'cancel' in pending} isDisabled={locked || busy} onPress={() => request('cancel', current.nextFingerprint, cancelCertificate)}>
              {'cancel' in pending ? 'Cancelling' : 'Cancel it'}
            </Button>
          </div>
        </>
      ) : (
        <>
          <span className="kp-field__help">To replace the certificate, prepare a new one first. Paired clients learn its fingerprint while this one stays in use, then you switch.</span>
          <div className="kp-row">
            <Button variant="secondary" isSending={'prepare' in pending} isDisabled={locked || busy} onPress={() => request('prepare', null, prepareCertificate)}>
              {'prepare' in pending ? 'Preparing' : 'Prepare a new certificate'}
            </Button>
          </div>
        </>
      )}

      <span className="kp-field__label">Token signing key</span>
      <span className="kp-row">
        <code className="kp-id">{current.signingKeyId}</code>
        <span className="kp-type-caption" style={{ color: 'var(--text-tertiary)' }}>
          {current.signingKeySince ? `Since ${dateFormat.format(new Date(current.signingKeySince))}` : 'Since this server was set up'}
        </span>
      </span>
      <span className="kp-field__help">
        {retiredUntil
          ? `The key it replaced is still accepted until ${retiredUntil}, when the last token it signed has run out.`
          : 'Rotating it logs no client out: tokens signed with the old key keep working until they run out, within 35 minutes.'}
      </span>
      <div className="kp-row">
        <Button variant="secondary" isSending={'rotate' in pending} isDisabled={locked || busy} onPress={() => request('rotate', current.signingKeyId, rotateSigningKey)}>
          {'rotate' in pending ? 'Rotating' : 'Rotate signing key'}
        </Button>
      </div>
      {failure && <p className="kp-error">Not done: {failure}</p>}

      <ModalDialog isOpen={confirmSwitch} onOpenChange={setConfirmSwitch} title="Switch to the new certificate?">
        {missing.length === 0 ? (
          <p className="kp-note">Every client has the new fingerprint and moves to it on its own. The old certificate is gone after this.</p>
        ) : (
          <p className="kp-note kp-error">
            {names(missing)} {missing.length === 1 ? 'has' : 'have'} not received the new fingerprint and will refuse this server after the switch. Pair{' '}
            {missing.length === 1 ? 'it' : 'them'} again, or enter the new fingerprint under Configuration on {missing.length === 1 ? 'that machine' : 'those machines'}.
          </p>
        )}
        <div className="kp-row" style={{ justifyContent: 'flex-end' }}>
          <Button variant="secondary" slot="close">
            Cancel
          </Button>
          <Button
            variant={missing.length === 0 ? 'primary' : 'danger'}
            isDisabled={locked}
            onPress={() => {
              setConfirmSwitch(false);
              request('switch', current.nextFingerprint, switchCertificate);
            }}
          >
            Switch
          </Button>
        </div>
      </ModalDialog>
    </section>
  );
}
