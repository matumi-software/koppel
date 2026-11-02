import { useState } from 'react';
import { ActivityFeed, Button, Field, SegmentedControl, Switch } from '@koppel/react-ui';
import { toEntries, useActivity } from '@/activity/activity-context';
import type { KoppelRole } from '@/api/settings';
import { Frame } from '@/shell/frame';
import { useMachineName } from '@/shared/use-machine-name';
import { useFrame } from '@/shell/frame-context';
import { useSettings, type SettingStatus } from './use-settings';

const roles = [
  { value: 'server', label: 'Server' },
  { value: 'client', label: 'Client' },
  { value: 'both', label: 'Both' },
] as const;

/** What this machine does and how it starts. Every change waits for the server. */
export function MachinePage() {
  const host = useMachineName();
  const { locked } = useFrame();
  const { items } = useActivity();
  const { query, status, requestChange } = useSettings();

  const aside = (
    <>
      <h2 className="kp-type-heading" style={{ margin: 0 }}>
        Activity
      </h2>
      <ActivityFeed entries={toEntries(items)} empty="Changes show here when the server confirms or rejects them." />
    </>
  );

  if (!query.data) {
    return (
      <Frame title="This machine" aside={aside}>
        <p className="kp-note">{query.isError ? 'Could not load settings. Check that Koppel is running, then reload.' : 'Loading settings…'}</p>
      </Frame>
    );
  }

  const role = status('role')!;
  const share = status('shareOnStartup')!;
  const roleNote = role.pendingTarget ? 'Saving…' : role.failure ? `Not saved: ${role.failure}` : role.saved ? 'Saved' : undefined;

  return (
    <Frame title="This machine" lede={`${host}: what it does on the network and how it starts.`} aside={aside}>
      <section className="kp-card">
        <div className="kp-field">
          <span className="kp-field__label">Role</span>
          <div className="kp-row">
            <SegmentedControl<KoppelRole>
              label="Role"
              options={roles}
              value={role.confirmed}
              pendingValue={role.pendingTarget}
              isDisabled={locked}
              onChange={(value) => requestChange('role', value)}
            />
            {roleNote && <span className={role.failure ? 'kp-error kp-type-caption' : 'kp-field__help'}>{roleNote}</span>}
          </div>
          <span className="kp-field__help">A server shares the devices plugged into it. A client attaches devices from a server.</span>
        </div>
        {role.confirmed !== 'client' && (
          <>
            <Switch
              label="Share saved devices on startup"
              confirmed={share.confirmed}
              pendingTarget={share.pendingTarget}
              failure={share.failure}
              note={share.saved ? 'Saved' : undefined}
              isDisabled={locked}
              onRequestChange={(value) => requestChange('shareOnStartup', value)}
            />
            <ListenPortField key={status('listenPort')!.confirmed} locked={locked} status={status('listenPort')!} onSave={(port) => requestChange('listenPort', port)} />
          </>
        )}
      </section>
    </Frame>
  );
}

function ListenPortField({ locked, status, onSave }: { locked: boolean; status: SettingStatus<'listenPort'>; onSave: (port: number) => void }) {
  const [draft, setDraft] = useState(String(status.confirmed));
  const isPending = status.pendingTarget !== undefined;
  const help = status.failure ? `Not saved: ${status.failure}` : status.saved ? 'Saved' : 'USB traffic port. Encrypted, and only registered clients get in.';
  return (
    <div style={{ maxWidth: 360 }}>
      <Field
        label="Listen port"
        mono
        value={draft}
        onChange={setDraft}
        isDisabled={locked || isPending}
        state={isPending ? 'pending' : status.failure ? 'invalid' : undefined}
        help={help}
        action={
          <Button variant="secondary" isSending={isPending} isDisabled={locked || draft === String(status.confirmed)} onPress={() => onSave(Number(draft))}>
            Save
          </Button>
        }
      />
    </div>
  );
}
