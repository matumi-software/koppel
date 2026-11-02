import { useState } from 'react';
import { Button, Field, Panel, RoleCards, Setting, SettingList, Switch } from '@koppel/react-ui';
import { Frame } from '@/shell/frame';
import { useMachineName } from '@/shared/use-machine-name';
import { useFrame } from '@/shell/frame-context';
import { roleOptions } from './roles';
import { useSettings, type SettingStatus } from './use-settings';

/** What this machine does and how it starts. Every change waits for the server. */
export function MachinePage() {
  const host = useMachineName();
  const { locked } = useFrame();
  const { query, status, requestChange } = useSettings();

  if (!query.data) {
    return (
      <Frame title="This machine">
        <p className="kp-note">{query.isError ? 'Could not load settings. Check that Koppel is running, then reload.' : 'Loading settings…'}</p>
      </Frame>
    );
  }

  const role = status('role')!;
  const share = status('shareOnStartup')!;

  return (
    <Frame title="This machine" lede={`${host}: what it does on the network and how it starts.`}>
      <Panel title="Role" meta={<SavedNote status={role} />}>
        <div className="kp-panel__body">
          <RoleCards
            label="Role"
            options={roleOptions}
            value={role.confirmed}
            pendingValue={role.pendingTarget}
            isDisabled={locked}
            onChange={(value) => value !== role.confirmed && requestChange('role', value)}
          />
        </div>
      </Panel>

      {role.confirmed !== 'client' && (
        <Panel title="Sharing">
          <SettingList label="Sharing">
            <Setting
              title="Share saved devices on startup"
              description="When Koppel starts, the devices you shared before are shared again."
              control={
                <Switch
                  label="Share saved devices on startup"
                  labelHidden
                  confirmed={share.confirmed}
                  pendingTarget={share.pendingTarget}
                  failure={share.failure}
                  note={share.saved ? 'Saved' : undefined}
                  isDisabled={locked}
                  onRequestChange={(value) => requestChange('shareOnStartup', value)}
                />
              }
            />
            <ListenPortSetting
              key={status('listenPort')!.confirmed}
              locked={locked}
              status={status('listenPort')!}
              onSave={(port) => requestChange('listenPort', port)}
            />
          </SettingList>
        </Panel>
      )}
    </Frame>
  );
}

/** Saving, Saved or why not, for a setting the panel holds. Nothing until the user changes it. */
function SavedNote({ status }: { status: SettingStatus<'role'> }) {
  if (status.pendingTarget !== undefined) return <span className="kp-panel__note" data-state="pending">Saving…</span>;
  if (status.failure) return <span className="kp-panel__note" data-state="fault">Not saved: {status.failure}</span>;
  return status.saved ? <span className="kp-panel__note">Saved</span> : null;
}

function ListenPortSetting({ locked, status, onSave }: { locked: boolean; status: SettingStatus<'listenPort'>; onSave: (port: number) => void }) {
  const [draft, setDraft] = useState(String(status.confirmed));
  const isPending = status.pendingTarget !== undefined;
  const help = status.failure ? `Not saved: ${status.failure}` : status.saved ? 'Saved' : undefined;
  return (
    <Setting
      title="Listen port"
      description="The port USB traffic arrives on. Encrypted, and only registered clients get in."
      control={
        <Field
          label="Listen port"
          labelHidden
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
      }
    />
  );
}
