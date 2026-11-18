import { useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { Plus, PlugZap, Settings, Trash2 } from 'lucide-react';
import { Callout, ConfirmDialog, IconButton, Inspector, PageHeader, Property, PropertySection, toast, StatusDot, Table, Tile, TileStack } from '@heartwood/ui';
import type { ServerSettings, ServerView } from '@/api/servers';
import { testServer } from '@/api/servers';
import { useClients } from '@/clients/use-clients';
import { useClientDevices } from '@/devices/use-devices';
import { reasonOf } from '@/errors';
import { systemInfoOptions } from '@/queries';
import { useMachineName } from '@/shared/use-machine-name';
import { isClientRole, isServerRole } from '@/settings/roles';
import { useSettings } from '@/settings/use-settings';
import { useFrame, useInspector } from '@/shell/frame-context';
import { Loading, QueryGate } from '@/shell/loading';
import { AddServerForm } from './add-server-form';
import { serverLabel, useServerChanges, useServers } from './use-servers';

const linkWord = { connected: 'Connected', connecting: 'Reconnecting', error: 'Cannot reach', unconfigured: 'Not linked' } as const;
const linkTone = { connected: 'success', connecting: 'pending', error: 'danger', unconfigured: 'neutral' } as const;

const linkFields = ['reconnect', 'retryEverySeconds', 'giveUpAfterSeconds'] as const;
type LinkField = (typeof linkFields)[number];
type Link = Required<Pick<ServerSettings, LinkField>>;

const portOk = (port: number | null) => port === null || (Number.isInteger(port) && port >= 1 && port <= 65535);

/**
 * Where this computer attaches from and what it shares: the servers with their link state, adding and
 * removing one, and a staged panel at the right (Reconnect, Retry every, Give up after, Port) with
 * Revert and Apply. Accept from applies at once. Every change is pending until the push brings it back.
 */
export function ConnectionPage() {
  const { locked, narrow } = useFrame();
  const host = useMachineName();
  const settings = useSettings();
  const role = settings.query.data?.role;
  const client = isClientRole(role);
  const server = isServerRole(role);
  const serversQuery = useServers(client);
  const devices = useClientDevices(client).data;
  const clients = useClients(server).data;
  const info = useQuery(systemInfoOptions()).data;
  const changes = useServerChanges();

  const [adding, setAdding] = useState(false);
  const [selected, setSelected] = useState<string>();
  // Where the panel is a sheet it is not mounted until a person asks for it (a closed one leaves a rail that takes a
  // phone's width) and is mounted open; where it is a column Heartwood holds its state.
  const [sheetOpen, setSheetOpen] = useState(false);
  const pick = (id: string) => {
    setSelected(id);
    setSheetOpen(true);
  };
  const [drafts, setDrafts] = useState<Record<string, Partial<Link>>>({});
  const [portDraft, setPortDraft] = useState<number | null>();
  const [removing, setRemoving] = useState<ServerView>();
  const [testing, setTesting] = useState<string>();
  const test = useMutation({
    mutationFn: (id: string) => testServer(id),
    onMutate: (id) => setTesting(id),
    onSettled: () => setTesting(undefined),
    onSuccess: (result, id) => {
      const name = serversQuery.data?.servers.find((s) => s.id === id);
      toast({ title: result.ok ? `${name ? serverLabel(name) : 'The server'} answers` : 'The connection test failed', text: result.message, tone: result.ok ? 'success' : 'danger' });
    },
    onError: (error) => toast({ title: 'The connection test failed', text: reasonOf(error, 'the request failed.'), tone: 'danger' }),
  });

  const list = serversQuery.data?.servers ?? [];
  const port = settings.status('networkPort');
  const accept = settings.status('networkAcceptFrom');

  const sel = selected && (selected === 'self' ? server : list.some((s) => s.id === selected)) ? selected : client && list[0] ? list[0].id : 'self';
  const selServer = list.find((s) => s.id === sel);

  // What a server shows: what it saved, then a change still waiting for its push, then what is being edited.
  const waiting = (s: ServerView): Partial<Link> => {
    const action = changes.pending[s.id];
    return action?.type === 'change' ? action.change : {};
  };
  const base = (s: ServerView): Link => ({ reconnect: s.reconnect, retryEverySeconds: s.retryEverySeconds, giveUpAfterSeconds: s.giveUpAfterSeconds, ...waiting(s) });
  const draftOf = (s: ServerView): Partial<Link> => {
    const before = base(s);
    const edit = drafts[s.id] ?? {};
    return Object.fromEntries(linkFields.filter((f) => f in edit && edit[f] !== before[f]).map((f) => [f, edit[f]])) as Partial<Link>;
  };
  const shown = (s: ServerView): Link => ({ ...base(s), ...drafts[s.id] });
  const portNow = portDraft !== undefined ? portDraft : (port?.pendingTarget ?? port?.confirmed ?? null);
  const portChanged = portDraft !== undefined && portDraft !== (port?.pendingTarget ?? port?.confirmed ?? null);
  const dirty = portChanged || list.some((s) => Object.keys(draftOf(s)).length > 0);
  const invalid =
    (portDraft !== undefined && !portOk(portDraft)) ||
    list.some((s) => Object.values(shown(s)).some((v) => typeof v === 'number' && (!Number.isFinite(v) || v < 1)));

  const revert = () => {
    setDrafts({});
    setPortDraft(undefined);
  };
  const apply = () => {
    for (const s of list) {
      const fields = draftOf(s);
      if (Object.keys(fields).length) changes.change(s, fields);
    }
    if (portChanged) settings.requestChange('networkPort', portDraft ?? null);
    revert();
  };
  const edit = (s: ServerView, patch: Partial<Link>) => setDrafts((d) => ({ ...d, [s.id]: { ...d[s.id], ...patch } }));

  const portFixed = info?.networkPortFixed ?? false;
  const inUse = info?.httpsPort;
  const portNote = portFixed
    ? 'A startup option sets the port, so the saved one is not used.'
    : port?.pendingTarget !== undefined || (port?.confirmed != null && port.confirmed !== inUse)
      ? 'Applies after Koppel restarts.'
      : undefined;
  const portFailure = port?.failure;

  const here = (s: ServerView) => (devices?.servers.find((d) => d.id === s.id)?.devices ?? []).filter((e) => e.attachedHere).map((e) => e.device.name);
  const linkOf = (s: ServerView) => devices?.servers.find((d) => d.id === s.id)?.link ?? 'connecting';

  // Where the panel is a sheet over the page it opens when a person asks (a Settings button), never on arrival.
  // The form is about a server that is not saved yet, so the panel of another one stays shut while it is open.
  const first = client && list.length === 0;
  const showForm = client && (adding || first);
  const inspector = !settings.query.data || showForm || (narrow && !sheetOpen) ? null : selServer ? (
    <Inspector title="Settings" subject={serverLabel(selServer)} dirty={dirty} staged={{ onRevert: revert, onApply: apply, invalid: locked || invalid }} storageKey="koppel.connection" open={narrow ? true : undefined} onOpenChange={narrow ? setSheetOpen : undefined}>
      {/* On a phone the panel is a sheet over the page, where the failure below the header cannot be seen. */}
      {narrow && changes.failures[selServer.id] ? (
        <div className="kp-sheet-note">
          <Callout tone="danger" title="Not saved">
            {changes.failures[selServer.id]}
          </Callout>
        </div>
      ) : null}
      <PropertySection title="Link">
        <Property label="Address" type="readonly" copy value={selServer.serverUrl?.replace(/^https:\/\//, '') ?? ''} />
        <Property label="Reconnect" type="boolean" value={shown(selServer).reconnect} resetTo={selServer.reconnect} onChange={(v) => edit(selServer, { reconnect: !!v })} />
        <Property
          label="Retry every"
          type="number"
          unit="s"
          min={1}
          value={shown(selServer).retryEverySeconds}
          resetTo={selServer.retryEverySeconds}
          disabled={!shown(selServer).reconnect}
          hint={shown(selServer).reconnect ? undefined : 'Turn Reconnect on to retry.'}
          onChange={(v) => edit(selServer, { retryEverySeconds: Number(v) })}
        />
        <Property label="Give up after" type="number" unit="s" min={1} value={shown(selServer).giveUpAfterSeconds} resetTo={selServer.giveUpAfterSeconds} onChange={(v) => edit(selServer, { giveUpAfterSeconds: Number(v) })} />
      </PropertySection>
      <PropertySection title="Info" defaultOpen={false}>
        <Property label="Fingerprint" type="readonly" copy value={selServer.serverFingerprint ?? ''} />
        <Property label="Linked with" type="readonly" value={selServer.linkedWith === 'key' ? 'Server key' : selServer.linkedWith === 'code' ? 'Pairing code' : (selServer.linkedWith ?? '')} />
      </PropertySection>
    </Inspector>
  ) : server ? (
    <Inspector title="Settings" subject={host} dirty={dirty} staged={{ onRevert: revert, onApply: apply, invalid: locked || invalid }} storageKey="koppel.connection" open={narrow ? true : undefined} onOpenChange={narrow ? setSheetOpen : undefined}>
      <PropertySection title="Sharing">
        <Property
          label="Port"
          type="number"
          min={1}
          max={65535}
          nullable
          emptyLabel={inUse ? `Default (${inUse})` : 'Default'}
          disabled={portFixed}
          value={portNow}
          resetTo={port?.confirmed ?? null}
          note={portFailure ? `Not saved: ${portFailure}` : portNote}
          noteTone={portFailure ? 'danger' : undefined}
          onChange={(v) => setPortDraft(v === null || v === undefined || v === '' ? null : Number(v))}
        />
        <Property
          label="Accept from"
          type="choice"
          select
          options={[
            { id: 'lan', label: 'This network only' },
            { id: 'any', label: 'Any network' },
          ]}
          value={accept?.pendingTarget ?? accept?.confirmed}
          disabled={locked}
          note={accept?.failure ? `Not saved: ${accept.failure}` : accept?.pendingTarget !== undefined ? 'Saving' : undefined}
          noteTone={accept?.failure ? 'danger' : undefined}
          onChange={(v) => settings.requestChange('networkAcceptFrom', v === 'lan' ? 'lan' : 'any')}
        />
      </PropertySection>
      <PropertySection title="Info" defaultOpen={false}>
        <Property label="Fingerprint" type="readonly" copy value={info?.fingerprint ?? ''} />
      </PropertySection>
    </Inspector>
  ) : null;
  useInspector(inspector);

  if (!settings.query.data) return <Loading />;
  if (client && !serversQuery.data) return <QueryGate query={serversQuery} />;

  const failed = list.filter((s) => changes.failures[s.id]);
  const description = first
    ? 'Not paired with a server yet.'
    : `${client ? `Attaches devices from ${list.length === 1 ? serverLabel(list[0]) : `${list.length} servers`}` : 'Shares its devices'}${client && server ? `; shares its own on port ${inUse ?? 'the default'}` : ''}.`;

  return (
    <div className="kp-page">
      <PageHeader
        title={`${host} on the network`}
        description={description}
        actions={client && !showForm ? <IconButton icon={Plus} label="Add a server" variant="primary" size="md" isDisabled={locked} onPress={() => setAdding(true)} /> : undefined}
      />
      {failed.map((s) => (
        <Callout key={s.id} tone="danger" title={`${serverLabel(s)} is not saved`}>
          {changes.failures[s.id]}
        </Callout>
      ))}
      <TileStack>
        {showForm ? (
          <Tile title={first ? 'Pair with a server' : 'Add a server'} padded>
            <AddServerForm first={first} onClose={() => setAdding(false)} />
          </Tile>
        ) : null}
        {client && list.length ? (
          <Tile title="Servers" meta={String(list.length)} className="kp-picked">
            <Table
              label="Servers this computer attaches from"
              density="compact"
              columns={[
                { id: 'name', label: 'Server', isRowHeader: true },
                { id: 'here', label: 'Attached here' },
                { id: 'link', label: 'Link', width: 140 },
                { id: 'act', label: 'Actions', width: 136, actions: true },
                // A phone has no room for what is attached here (Remote devices lists it), and shows the link under
                // the server's name, so the pinned Actions never sit over it and a long address wraps.
              ].filter((column) => !narrow || (column.id !== 'here' && column.id !== 'link'))}
              rows={list.map((s) => ({ id: s.id }))}
              onRowAction={(id) => pick(String(id))}
              current={sel}
              renderCell={(row, col) => {
                const s = list.find((x) => x.id === row.id)!;
                const action = changes.pending[s.id];
                const state = action ? (
                  <StatusDot tone="pending">{action.type === 'remove' ? 'Removing' : 'Applying'}</StatusDot>
                ) : (
                  <StatusDot tone={linkTone[linkOf(s)]}>{linkWord[linkOf(s)]}</StatusDot>
                );
                if (col.id === 'name') {
                  return narrow ? (
                    <span className="kp-cell-stack">
                      <span className="kp-wrap-anywhere">{serverLabel(s)}</span>
                      {state}
                    </span>
                  ) : (
                    serverLabel(s)
                  );
                }
                if (col.id === 'here') return here(s).join(', ') || <span className="kp-muted">Nothing</span>;
                if (col.id === 'link') return state;
                return (
                  <span className="kp-line">
                    <IconButton icon={PlugZap} label={`Test the connection to ${serverLabel(s)}`} size="sm" isDisabled={locked || testing === s.id} onPress={() => test.mutate(s.id)} />
                    <IconButton icon={Settings} label={`Settings for ${serverLabel(s)}`} description="Opens them in the panel at the right." size="sm" onPress={() => pick(s.id)} />
                    <IconButton icon={Trash2} label={`Remove ${serverLabel(s)}`} danger isDisabled={locked || !!action} onPress={() => setRemoving(s)} />
                  </span>
                );
              }}
            />
          </Tile>
        ) : null}
        {server ? (
          <Tile title="This server" className="kp-picked">
            <Table
              label="This server"
              density="compact"
              columns={[
                { id: 'name', label: 'Server', isRowHeader: true },
                { id: 'addr', label: 'Address' },
                { id: 'clients', label: 'Clients', width: 120 },
                { id: 'act', label: 'Actions', width: 80, actions: true },
              ]}
              rows={[{ id: 'self' }]}
              onRowAction={() => pick('self')}
              current={sel}
              renderCell={(_row, col) => {
                if (col.id === 'name') return host;
                if (col.id === 'addr') return <span className="hw-mono">{`${host}:${inUse ?? ''}`}</span>;
                if (col.id === 'clients') return clients ? `${clients.length} linked` : '';
                return <IconButton icon={Settings} label={`Settings for ${host}`} description="Opens them in the panel at the right." size="sm" onPress={() => pick('self')} />;
              }}
            />
          </Tile>
        ) : null}
      </TileStack>
      {removing ? (
        <ConfirmDialog
          title={`Remove ${serverLabel(removing)}?`}
          body={`Koppel detaches the devices it has attached from ${serverLabel(removing)} and forgets the server. To use it again, add it again with a pairing code or its key.`}
          confirmLabel="Remove"
          danger
          onConfirm={() => {
            if (locked) return;
            changes.remove(removing);
            setRemoving(undefined);
          }}
          onCancel={() => setRemoving(undefined)}
        />
      ) : null}
    </div>
  );
}
