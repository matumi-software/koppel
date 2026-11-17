import { useQuery } from '@tanstack/react-query';
import { Pencil, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { Button, Callout, CopyValue, Dialog, IconButton, PageHeader, SetPasswordDialog, SettingRow, SettingsList, showToast, Tile, TileStack, useChangePassword } from '@heartwood/ui';
import { changePassword } from '@/api/auth';
import type { ClientView } from '@/api/server';
import { minPasswordLength } from '@/auth/password-rules';
import { reasonOf } from '@/errors';
import type { ServerKeysAction } from '@/pending/domains';
import { usePending } from '@/pending/pending-context';
import { clientsOptions, serverKeysOptions, settingsOptions, systemInfoOptions } from '@/queries';
import { isServerRole } from '@/settings/roles';
import { useAnnounce } from '@/shared/announce-context';
import { useFrame } from '@/shell/frame-context';
import { Loading } from '@/shell/loading';

const names = (clients: ClientView[]) => clients.map((c) => c.name).join(', ');

/** The admin password's own change: it completes on the answer, since a password is never pushed back. */
function usePasswordChange() {
  const announce = useAnnounce();
  return useChangePassword(async (next, current) => {
    try {
      await changePassword(current, next);
    } catch (error) {
      const reason = reasonOf(error, 'The server did not answer.');
      announce(`The password was not changed. ${reason}`);
      return reason;
    }
    announce('The password was changed. Other browsers are signed out.');
    showToast({ title: 'Password changed. Other browsers are signed out.', tone: 'success' });
  });
}

/**
 * The certificate clients pin, and its handover: prepare a new one so clients learn its fingerprint,
 * then switch. Each step is pending until the server pushes the new keys (the confirm rule).
 */
function ServerCertificate({ locked }: { locked: boolean }) {
  const keys = useQuery(serverKeysOptions());
  const clientList = useQuery(clientsOptions());
  const { pending, failures, request } = usePending('serverKeys');
  const [confirm, setConfirm] = useState<'switch' | 'discard' | null>(null);

  const current = keys.data;
  if (!current) return null;

  const clients = clientList.data ?? [];
  const missing = clients.filter((c) => !c.hasNextCertificate);
  const doing = (type: ServerKeysAction['type']) => type in pending;
  const busy = Object.keys(pending).length > 0;
  const failure = Object.values(failures)[0];
  const next = current.nextFingerprint;
  const off = locked || busy;

  const ready = !clients.length
    ? 'Ready. No clients are paired yet.'
    : `Ready. ${clients.length - missing.length} of ${clients.length} clients have it${missing.length ? `; ${names(missing)} ${missing.length === 1 ? 'has' : 'have'} not connected since it was made` : ''}.`;

  return (
    <>
      <Tile title="Server certificate">
        <SettingsList label="Server certificate">
          <SettingRow title="Fingerprint" description="Clients check it on every connection." control={<CopyValue value={current.fingerprint} label="Fingerprint" />} />
          {next ? <SettingRow title="New fingerprint" description="Enter it on a client that did not get it." control={<CopyValue value={next} label="New fingerprint" />} /> : null}
          <SettingRow
            title="New certificate"
            description={next ? ready : 'Make one ahead of time, so clients learn it before you switch.'}
            control={
              next ? (
                <span className="kp-line">
                  <IconButton icon={Trash2} label="Discard the new certificate" variant="secondary" danger quiet isPending={doing('cancel')} isDisabled={off} onPress={() => setConfirm('discard')} />
                  <Button variant="secondary" aria-label="Switch to the new certificate" isPending={doing('switch')} isDisabled={off} onPress={() => setConfirm('switch')}>
                    {doing('switch') ? 'Switching' : 'Switch to it'}
                  </Button>
                </span>
              ) : (
                <Button variant="secondary" aria-label="Prepare a new certificate" isPending={doing('prepare')} isDisabled={off} onPress={() => request({ type: 'prepare' })}>
                  {doing('prepare') ? 'Preparing' : 'Prepare'}
                </Button>
              )
            }
          />
        </SettingsList>
      </Tile>

      {failure && (
        <Callout tone="danger" title="Not done">
          {failure}
        </Callout>
      )}

      {confirm === 'discard' && (
        <Dialog
          title="Discard the new certificate?"
          size="sm"
          role="alertdialog"
          hasCloseButton={false}
          onClose={() => setConfirm(null)}
          footer={(close) => (
            <>
              <Button variant="secondary" autoFocus onPress={close}>
                Cancel
              </Button>
              <Button
                variant="destructive"
                isDisabled={locked}
                onPress={() => {
                  close();
                  request({ type: 'cancel' });
                }}
              >
                Discard
              </Button>
            </>
          )}
        >
          Clients that already learned it forget it. You can prepare another one later.
        </Dialog>
      )}
      {confirm === 'switch' && (
        <Dialog
          title="Switch to the new certificate?"
          size="sm"
          role="alertdialog"
          hasCloseButton={false}
          onClose={() => setConfirm(null)}
          footer={(close) => (
            <>
              <Button variant="secondary" autoFocus onPress={close}>
                Cancel
              </Button>
              <Button
                variant={missing.length ? 'destructive' : 'primary'}
                isDisabled={locked}
                onPress={() => {
                  close();
                  request({ type: 'switch', nextFingerprint: next });
                }}
              >
                Switch certificate
              </Button>
            </>
          )}
        >
          {missing.length === 0
            ? 'Every client has the new fingerprint and moves to it on its own. The old certificate is gone after this.'
            : `${names(missing)} ${missing.length === 1 ? 'has' : 'have'} not received the new fingerprint and will refuse this server after the switch. Pair ${missing.length === 1 ? 'it' : 'them'} again, or enter the new fingerprint on ${missing.length === 1 ? 'that computer' : 'those computers'}.`}
        </Dialog>
      )}
    </>
  );
}

/**
 * Who can change this computer and how clients know they reached it: the server's certificate and
 * its handover (server role only), and the admin password.
 */
export function SecurityPage() {
  const { locked } = useFrame();
  const settings = useQuery(settingsOptions());
  const info = useQuery(systemInfoOptions());
  const password = usePasswordChange();

  if (!settings.data) return <Loading />;

  const name = info.data?.name ?? window.location.hostname;
  const server = isServerRole(settings.data.role);

  return (
    <div className="kp-page">
      <PageHeader title="Security" description={server ? `Who can change ${name}, and how clients know they reached it.` : `Who can change ${name}.`} />
      <TileStack>
        {server && <ServerCertificate locked={locked} />}
        <Tile title="Sign-in">
          <SettingsList label="Sign-in">
            <SettingRow
              title="Admin password"
              description="Anyone with it can change what this computer shares."
              control={<IconButton icon={Pencil} label="Change password" isDisabled={locked} onPress={password.show} />}
            />
          </SettingsList>
        </Tile>
      </TileStack>
      {password.open && (
        <SetPasswordDialog
          userName="admin"
          change
          askCurrent
          minLength={minPasswordLength}
          note="Other browsers signed in with the old password are signed out. This one stays in."
          currentError={password.error}
          isPending={password.pending}
          onSet={(next, current) => void password.submit(next, current)}
          onCancel={password.close}
        />
      )}
    </div>
  );
}
