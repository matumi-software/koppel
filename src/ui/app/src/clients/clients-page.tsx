import { useState } from 'react';
import { RefreshCw, Trash2 } from 'lucide-react';
import { Button, Callout, DrillView, IconButton, PageHeader, PairingCode, SettingRow, SettingsList, StateLabel, StatusDot, Table, Tile, TileStack, useDrill } from '@heartwood/ui';
import { states } from '@/brand';
import { useServerDevices } from '@/devices/use-devices';
import { useMachineName } from '@/shared/use-machine-name';
import { useNow } from '@/shared/use-now';
import { useFrame } from '@/shell/frame-context';
import { Loading } from '@/shell/loading';
import { ClientPage, RevokeDialog } from './client-page';
import { lastSeen, linkedWord } from './client-words';
import { PairingRequestsTile, PairingWindowRow } from './pairing-requests';
import { AddClientDialog, RotateSecretDialog } from './secret-dialogs';
import { ServerKey } from './server-key';
import { useClients, useRevoke } from './use-clients';
import { useEnrollmentCode } from './use-enrollment';

/** The pairing code with its countdown, counted from what is left at the moment it first shows. */
function LinkCode({ code }: { code: { code: string; expiresAt: string } }) {
  const [seconds] = useState(() => Math.max(1, Math.round((Date.parse(code.expiresAt) - Date.now()) / 1000)));
  return <PairingCode code={code.code} seconds={seconds} note="On the new computer, go to Connection, enter this computer's address and this code." />;
}

/**
 * The computers registered on this server: link a new one with a pairing code or the server key, see
 * what each uses, and open one to revoke it or replace its secret. Revoking is pending until the list drops it.
 */
export function ClientsPage() {
  const { locked, narrow } = useFrame();
  const now = useNow();
  const soon = useNow(5_000);
  const host = useMachineName();
  const clients = useClients();
  const devices = useServerDevices().data?.devices;
  const enrollment = useEnrollmentCode();
  const { pending, failures } = useRevoke();
  const drill = useDrill<string>();
  const [rotating, setRotating] = useState<{ id: string; name: string }>();
  const [removing, setRemoving] = useState<{ id: string; name: string }>();
  const [adding, setAdding] = useState(false);

  if (!clients.data) return <Loading />;
  const list = clients.data;
  if (drill.top) {
    return (
      <DrillView dir={drill.dir} viewKey={`client-${drill.top}`}>
        <ClientPage clientId={drill.top} back={{ label: 'Clients', onPress: drill.back }} />
      </DrillView>
    );
  }

  const usingOf = (id: string) => (devices ?? []).filter((d) => d.state === 'attached' && d.attachedTo === id).map((d) => d.name);
  const failed = list.filter((c) => failures[c.id]);
  return (
    <DrillView dir={drill.dir} viewKey="clients">
      <div className="kp-page">
        <PageHeader title={`Clients of ${host}`} description={list.length ? `${list.length} linked.` : 'No computer is linked yet.'} />
        {failed.map((c) => (
          <Callout key={c.id} tone="danger" title={`${c.name} is not revoked`}>
            {failures[c.id]}
          </Callout>
        ))}
        <TileStack>
          <Tile
            title="Link a computer"
            padded
            action={enrollment.code ? <IconButton icon={RefreshCw} label="New code" description="The old code stops working." isDisabled={locked || enrollment.isPending} onPress={enrollment.open} /> : undefined}
          >
            <div className="kp-stack-3">
              {enrollment.code ? <LinkCode key={enrollment.code.code} code={enrollment.code} /> : null}
              {enrollment.code && Date.parse(enrollment.code.expiresAt) <= soon ? (
                <div className="kp-actions">
                  <Button variant="primary" icon={RefreshCw} isPending={enrollment.isPending} isDisabled={locked} onPress={enrollment.open}>
                    Show a new code
                  </Button>
                </div>
              ) : null}
              {enrollment.failure ? <Callout tone="danger" title="No new code">{enrollment.failure}</Callout> : null}
              <SettingsList label="Link a computer">
                {enrollment.code ? null : (
                  <SettingRow
                    title="New pairing code"
                    description="It works once, for 60 seconds. A new code replaces the old one."
                    control={
                      <Button variant="primary" isPending={enrollment.isPending} isDisabled={locked} aria-label="Show a new pairing code" onPress={enrollment.open}>
                        Show
                      </Button>
                    }
                  />
                )}
                <SettingRow title="Server key" description="Links any computer, any time, without a code. Keep it private." control={<ServerKey revision={enrollment.keyRevision} />} />
                <PairingWindowRow />
                <SettingRow
                  title="Client ID and secret"
                  description="For a computer that cannot use a code or the key."
                  control={
                    <Button variant="secondary" isDisabled={locked} onPress={() => setAdding(true)}>
                      Add by hand
                    </Button>
                  }
                />
              </SettingsList>
            </div>
          </Tile>
          <PairingRequestsTile />
          <Tile title="Computers" meta={String(list.length)}>
            {list.length ? (
              <Table
                label={`Computers linked to ${host}`}
                density="compact"
                columns={[
                  { id: 'name', label: 'Computer', isRowHeader: true },
                  { id: 'via', label: 'Linked with', width: 140 },
                  { id: 'using', label: 'Using' },
                  { id: 'seen', label: 'Last seen', width: 160 },
                  { id: 'act', label: 'Actions', width: 100, actions: true },
                  // A phone has no room for all of them; the computer's own page has every fact.
                ].filter((column) => !narrow || !['via', 'seen'].includes(column.id))}
                rows={list.map((c) => ({ id: c.id, name: c.name }))}
                onRowAction={(id) => drill.open(String(id))}
                renderCell={(row, col) => {
                  const c = list.find((x) => x.id === row.id)!;
                  const using = usingOf(c.id);
                  if (col.id === 'via') return <span className="kp-muted">{linkedWord(c)}</span>;
                  if (col.id === 'using')
                    return using.length ? (
                      <span className="kp-line">
                        <StateLabel states={states} state="Attached" />
                        <span>{using.join(', ')}</span>
                      </span>
                    ) : (
                      <span className="kp-muted">Nothing</span>
                    );
                  if (col.id === 'seen') return <span className="kp-muted">{lastSeen(c, now)}</span>;
                  if (col.id === 'act')
                    return pending[c.id] ? (
                      <StatusDot tone="pending">Revoking</StatusDot>
                    ) : (
                      <span className="kp-line">
                        <IconButton icon={RefreshCw} label={`Rotate the secret of ${c.name}`} description="Its current secret stops working." isDisabled={locked} onPress={() => setRotating(c)} />
                        <IconButton icon={Trash2} label={`Revoke ${c.name}`} danger isDisabled={locked} onPress={() => setRemoving(c)} />
                      </span>
                    );
                  return c.name;
                }}
              />
            ) : (
              <p className="kp-muted kp-pad">No computer is linked yet. Show a pairing code and enter it on the other computer.</p>
            )}
          </Tile>
        </TileStack>
        {adding ? <AddClientDialog onClose={() => setAdding(false)} /> : null}
        {rotating ? <RotateSecretDialog client={rotating} onClose={() => setRotating(undefined)} /> : null}
        {removing ? <RevokeDialog client={removing} onClose={() => setRemoving(undefined)} /> : null}
      </div>
    </DrillView>
  );
}
