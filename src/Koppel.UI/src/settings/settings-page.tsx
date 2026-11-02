import { useState } from 'react';
import { Button, Field, Switch } from '@koppel/react-ui';
import { useHub } from '@/hub/hub-context';
import { useSettings, type SettingStatus } from './use-settings';

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

  const shareOnStartup = status('shareOnStartup')!;
  const requireTls = status('requireTls')!;

  return (
    <div className="app-stack">
      <h1 className="kp-type-display app-title">Settings</h1>

      <section className="app-section">
        <h2 className="kp-type-title app-section__title">This machine</h2>
        <Switch
          label="Share devices on startup"
          confirmed={shareOnStartup.confirmed}
          pendingTarget={shareOnStartup.pendingTarget}
          failure={shareOnStartup.failure}
          note={shareOnStartup.saved ? 'Saved' : undefined}
          isDisabled={locked}
          onRequestChange={(value) => requestChange('shareOnStartup', value)}
        />
        <Switch
          label="Require TLS for clients"
          confirmed={requireTls.confirmed}
          pendingTarget={requireTls.pendingTarget}
          failure={requireTls.failure}
          note={requireTls.saved ? 'Saved' : undefined}
          isDisabled={locked}
          onRequestChange={(value) => requestChange('requireTls', value)}
        />
        <ListenPortField
          key={status('listenPort')!.confirmed}
          locked={locked}
          status={status('listenPort')!}
          onSave={(port) => requestChange('listenPort', port)}
        />
      </section>
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
  const help = status.failure ? `Not saved: ${status.failure}` : status.saved ? 'Saved' : 'Clients connect to this port.';

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
