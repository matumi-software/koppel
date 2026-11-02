import { useState } from 'react';
import { Button, Field, SegmentedControl, Switch } from '@koppel/react-ui';
import type { KoppelRole } from '@/api/settings';
import { ConnectionSection } from '@/client/connection-section';
import { useHub } from '@/hub/hub-context';
import { ClientsSection } from '@/server/clients-section';
import { useSettings, type SettingStatus } from './use-settings';

const roles = [
  { value: 'server', label: 'Server' },
  { value: 'client', label: 'Client' },
  { value: 'both', label: 'Both' },
] as const;

export function SettingsPage() {
  const { state: link } = useHub();
  const { query, status, requestChange } = useSettings();
  const locked = link !== 'live';

  if (query.isPending) {
    return <p className="app-note">Loading settings…</p>;
  }
  if (query.isError) {
    return <p className="app-note">Could not load settings. Check that the server is running, then reload.</p>;
  }

  const role = status('role')!;
  const shareOnStartup = status('shareOnStartup')!;
  const isServer = role.confirmed !== 'client';
  const isClient = role.confirmed !== 'server';
  const roleNote = role.pendingTarget ? 'Saving…' : role.failure ? `Not saved: ${role.failure}` : role.saved ? 'Saved' : undefined;

  return (
    <div className="app-stack">
      <h1 className="kp-type-display app-title">Settings</h1>

      <section className="app-section">
        <h2 className="kp-type-title app-section__title">This machine</h2>
        <div className="app-kv">
          <span className="kp-field__label">Role</span>
          <div className="app-inline">
            <SegmentedControl<KoppelRole>
              label="Role"
              options={roles}
              value={role.confirmed}
              pendingValue={role.pendingTarget}
              isDisabled={locked}
              onChange={(value) => requestChange('role', value)}
            />
            {roleNote && <span className={role.failure ? 'app-error app-flush' : 'kp-field__help'}>{roleNote}</span>}
          </div>
          <span className="kp-field__help">A server shares the devices plugged into it. A client attaches devices from a server.</span>
        </div>
        {isServer && (
          <>
            <Switch
              label="Share saved devices on startup"
              confirmed={shareOnStartup.confirmed}
              pendingTarget={shareOnStartup.pendingTarget}
              failure={shareOnStartup.failure}
              note={shareOnStartup.saved ? 'Saved' : undefined}
              isDisabled={locked}
              onRequestChange={(value) => requestChange('shareOnStartup', value)}
            />
            <ListenPortField
              key={status('listenPort')!.confirmed}
              locked={locked}
              status={status('listenPort')!}
              onSave={(port) => requestChange('listenPort', port)}
            />
          </>
        )}
      </section>

      {isServer && <ClientsSection locked={locked} />}
      {isClient && <ConnectionSection locked={locked} />}
    </div>
  );
}

interface ListenPortFieldProps {
  locked: boolean;
  status: SettingStatus<'listenPort'>;
  onSave: (port: number) => void;
}

function ListenPortField({ locked, status, onSave }: ListenPortFieldProps) {
  const [draft, setDraft] = useState(String(status.confirmed));
  const isPending = status.pendingTarget !== undefined;
  const help = status.failure ? `Not saved: ${status.failure}` : status.saved ? 'Saved' : 'USB traffic port. Encrypted, and only registered clients get in.';

  return (
    <div className="app-row">
      <Field
        label="Listen port"
        mono
        value={draft}
        onChange={setDraft}
        isDisabled={locked || isPending}
        state={isPending ? 'pending' : status.failure ? 'invalid' : undefined}
        help={help}
        action={
          <Button
            variant="secondary"
            isSending={isPending}
            isDisabled={locked || draft === String(status.confirmed)}
            onPress={() => onSave(Number(draft))}
          >
            Save
          </Button>
        }
      />
    </div>
  );
}
